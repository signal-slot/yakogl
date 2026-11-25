// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The vertex stage's cube sampler and texture descriptors of the dEQP-GLES2
//! shapes of issue #7489 equal what Mesa's panfrost sets, read through the
//! draw's own resource tables (item vscube-pin, plan #406;
//! docs/es2-cts-failures.md section M.1 rows 5 and 11).
//!
//! The shapes, each drawn through the GL core on the real Mali back end over
//! a mock of its OSAL (one triangle into an 8x8 RGBA8 framebuffer object):
//!
//! - texture.vertex.cube.filtering.linear_mipmap_linear_{linear,nearest}_repeat
//!   and texture.vertex.cube.wrap.repeat_repeat (es2fVertexTextureTests.cpp,
//!   `VertexCubeTextureCase`): `tools/mali/testdata/pan_tex_ref/vcube.vert`
//!   and `.frag`, a 32x32 GL_RGB cube map with 6 levels, min
//!   LINEAR_MIPMAP_LINEAR, the case's mag filter, GL_REPEAT on s and t, at
//!   three directions on the +X face within 0.01 of its edge with +Y.
//!   `wrap.repeat_repeat` has the state of `..._linear_repeat` (the wrap
//!   group draws min LINEAR_MIPMAP_LINEAR, mag LINEAR, as the header of
//!   `pan_tex_ref/vcube.wrap.clamp_repeat.desc` says).
//! - shaders.texture_functions.vertex.texturecubelod
//!   (es2fShaderTextureFunctionTests.cpp, `texCubeMipmap`): the case's shader
//!   pair, a 256x256 GL_RGBA cube map with 9 levels, min
//!   LINEAR_MIPMAP_NEAREST, mag LINEAR, GL_REPEAT on s and t, the +Z face out
//!   to its edges, levels of detail -1, 4 and 9.
//!
//! The expected fields are not a recorded trace: they follow Mesa 26.2.1's
//! source (the tree `tools/mali/build_pan_shc.sh` extracts,
//! tools/mali/README-pan_shc.md), cited as `M:<file>:<line>` with `M:` =
//! `src/`:
//!
//! - the sampler, `panfrost_create_sampler_state`
//!   (M:gallium/drivers/panfrost/pan_cmdstream.c:201-260): `Magnify nearest`
//!   and `Minify nearest` from the image filters (:215-216), `Normalized
//!   Coordinates` (:218), LOD bias, minimum and maximum LOD from the CSO
//!   (:219-221; a GL texture's are 0, 0 and 1000, M:mesa/main/texobj.c:385-386,
//!   and the 13-bit `ulod` field holds at most 8191/256 = 31.996094,
//!   M:panfrost/genxml/v10.xml:1145-1147), the wraps (:223-225,
//!   `translate_tex_wrap` :105-137), the mipmap mode (:227-228,
//!   `pan_pipe_to_mipmode` :151-169: NEAREST is Nearest, LINEAR Trilinear),
//!   the compare function Never (:230, :142-148), `Seamless Cube Map` (:231),
//!   a zero border (:233-236), round-to-nearest-even cleared only when both
//!   filters are NEAREST (:243-245); the rest is the packer's default
//!   (v10.xml:1128-1154).
//! - the context's part: `CubeMapSeamless = _mesa_is_gles3(ctx)`
//!   (M:mesa/main/version.c:682-690), handed to every sampler
//!   (M:mesa/state_tracker/st_atom_sampler.c:71), and for a seamless cube map
//!   st/mesa replaces the three wraps by clamp-to-edge whatever GL's are
//!   (st_atom_sampler.c:84-94). dEQP-GLES2 asks for a GLES 2 context; Mesa
//!   gives its ES 3 one, so on Mesa the board's words are the seamless ones.
//!   In an ES 2.0 context (Mesa under MESA_GLES_VERSION_OVERRIDE=2.0) the
//!   flag is clear and GL's wraps stay: REPEAT on s and t as set, and GL's
//!   default REPEAT on r.
//! - the texture, `pan_sampled_texture_emit`
//!   (M:panfrost/lib/pan_texture.c:1272-1325): `Dimension` Cube (:1300), the
//!   format's hardware word (:1283, :1301; R8G8B8A8_UNORM is RGBA8 UNORM read
//!   RGBA, M:panfrost/lib/pan_format.c:508), one face's width and height
//!   (:1302-1303), `Sample count` 1 (:1307), the swizzle (:1308), `Texel
//!   interleave` for a modifier that is not linear (:1310), `Levels` (:1315),
//!   `Array size` the layer count over 6 (:1316, `pan_texture_get_array_size`
//!   :1198-1217), `Minimum LOD` 0 and `Maximum LOD` levels - 1 (:1321-1322).
//! - a stage that binds no sampler gets the workaround sampler
//!   (pan_cmdstream.c:2029-2031), the packer's default: seamless,
//!   clamp-to-edge, mipmap mode Nearest, maximum LOD 0.
//!
//! This file builds with and without `report-es2`: without it the context is
//! the ES 3 one (seamless, clamped), with it (the board's dEQP-GLES2 library)
//! the ES 2.0 one. Word 0 of each packed sampler is also given as a literal
//! (`c6a99901` es3 and `c6288801` es2 for the linear state, the words of the
//! drm-shim traces of item cube-ref), so a change of the packer shows too.
//!
//! Not Mesa's by design, and pinned as ours: the GL core keeps a GL_RGB map
//! in RGBA8 storage, so its format word is RGBA8 UNORM with the RGB1 swizzle
//! (Mesa: RGB8 UNORM, RGB1, pan_format.c:459; the sampled value is the
//! same); our storage is linear, so `Texel interleave` is clear and the
//! planes are Generic (Mesa's are AFBC on the board); `Surfaces` is an
//! address.
//!
//! The fragment stage's words of the same state pass on the board
//! (texture.filtering.cube.*, texture.mipmap.cube.*): the test also draws a
//! pair in which both stages read the map, and one in which only the
//! fragment stage does, and requires the vertex table's sampler and texture
//! to equal the fragment table's field for field (but `Surfaces`).
//!
//! The mock gives the test no descriptor reader, so the test finds the
//! draw's two resource tables in the buffers the mock handed out (seven
//! 16-byte entries on a 64-byte boundary, `panfrost_emit_resources`,
//! M:gallium/drivers/panfrost/pan_cmdstream.h:279-323; it zeroes every such
//! table before a draw). The vertex table is the one with attributes
//! (entry 1), the fragment table the other; entry 3 is the samplers, entry 4
//! the textures, and the texture's `Surfaces` its planes.
//!
//! Every assertion holds at the tip in both builds, so none is `#[ignore]`d:
//! the sampler and texture descriptors of these draws are Mesa's, and the
//! vertex stage's equal the fragment stage's. What issue #7489 sees on the
//! board is therefore not a word of these descriptors.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_vs_cube_edge_gl
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table,report-es2 --test mali_vs_cube_edge_gl
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali"))]

use core::ffi::{c_char, c_void};
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
use yakogl::gl::vertex::{glDisableVertexAttribArray, glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::mali::genxml::v10::{
    DescriptorType, GenericPlane, MipmapMode, PlaneType, Sampler, Texture, TextureDimension, WrapMode,
};
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

/// What one stage's table holds: its sampler (entry 3, the first one) and,
/// when it has a texture (entry 4), the texture and its planes.
#[derive(Debug)]
struct Stage {
    sampler: [u8; 32],
    texture: Option<(Texture, Vec<GenericPlane>)>,
}

fn stage_of(label: &str, t: &Table) -> Stage {
    let sampler: [u8; 32] = at_gpu(t[ENTRY_SAMPLER].0, 32).unwrap().try_into().unwrap();
    let texture = (t[ENTRY_TEXTURE].1 != 0).then(|| {
        assert_eq!(t[ENTRY_TEXTURE].1, 32, "{label}: one texture");
        assert_eq!(t[ENTRY_SAMPLER].1, 32, "{label}: one sampler");
        let raw: [u8; 32] = at_gpu(t[ENTRY_TEXTURE].0, 32).unwrap().try_into().unwrap();
        let tex = Texture::unpack(&raw).unwrap_or_else(|e| panic!("{label}: the texture does not unpack: {e:?}"));
        assert_eq!(tex.type_, DescriptorType::TEXTURE, "{label}");
        /* the planes: one a face and level (pan_texture.c:1074-1100) */
        let n = 6 * tex.levels as usize;
        let bytes = at_gpu(tex.surfaces, n * 32).unwrap_or_else(|| panic!("{label}: {n} planes at Surfaces"));
        let planes = bytes
            .as_chunks::<32>()
            .0
            .iter()
            .map(|p| GenericPlane::unpack(p).unwrap_or_else(|e| panic!("{label}: a plane: {e:?}")))
            .collect();
        (tex, planes)
    });
    Stage { sampler, texture }
}

/// Zero the tables, draw, and read the draw's vertex and fragment tables.
fn stages(label: &str, drew: impl FnOnce() -> (GLenum, u32)) -> (Stage, Stage) {
    tables(true);
    let (e, refused) = drew();
    assert_eq!((e, refused), (GL_NO_ERROR, 0), "{label}: the draw was refused or failed");
    let found = tables(false);
    let vs: Vec<&Table> = found.iter().filter(|t| t[ENTRY_ATTRIBUTE].1 != 0).collect();
    let fs: Vec<&Table> = found.iter().filter(|t| t[ENTRY_ATTRIBUTE].1 == 0).collect();
    assert_eq!((vs.len(), fs.len()), (1, 1), "{label}: one vertex and one fragment table, found {found:x?}");
    (stage_of(label, vs[0]), stage_of(label, fs[0]))
}

/* ---- Mesa's words ----------------------------------------------------------- */

/// The context's seamless cube maps: M:mesa/main/version.c:682-690.
const SEAMLESS: bool = !cfg!(feature = "report-es2");

/// The 13-bit `ulod` maximum the CSO's 1000 packs to (v10.xml:1147).
const MAX_LOD: f32 = 8191.0 / 256.0;

/// One cube state of the cases.
#[derive(Clone, Copy)]
struct Case {
    name: &'static str,
    size: i32,
    levels: i32,
    rgba: bool,
    min: GLenum,
    mag: GLenum,
    /// Word 0 of Mesa's packed sampler in its ES 3 and ES 2.0 contexts.
    word0: [u32; 2],
}

/// Mesa's sampler of a cube map with GL_REPEAT on s and t (module comment;
/// the lines are pan_cmdstream.c's).
fn mesa_sampler(c: &Case) -> Sampler {
    /* st_atom_sampler.c:84-94 with the flag; GL's s, t and default r without */
    let wrap = if SEAMLESS { WrapMode::CLAMP_TO_EDGE } else { WrapMode::REPEAT };
    let (min_nearest, mipmap) = match c.min {
        GL_LINEAR_MIPMAP_LINEAR => (false, MipmapMode::TRILINEAR),
        GL_LINEAR_MIPMAP_NEAREST => (false, MipmapMode::NEAREST),
        m => panic!("min filter {m:#x}"),
    };
    let mag_nearest = c.mag == GL_NEAREST;
    Sampler {
        /* :223-225 */
        wrap_mode_s: wrap,
        wrap_mode_t: wrap,
        wrap_mode_r: wrap,
        /* :243-245 */
        round_to_nearest_even: !(mag_nearest && min_nearest),
        /* :231 */
        seamless_cube_map: SEAMLESS,
        /* :218 */
        normalized_coordinates: true,
        /* :215-216 */
        magnify_nearest: mag_nearest,
        minify_nearest: min_nearest,
        /* :227-228 */
        mipmap_mode: mipmap,
        /* :219-221 */
        lod_bias: 0.0,
        minimum_lod: 0.0,
        maximum_lod: MAX_LOD,
        ..Default::default()
    }
}

/// The sampler of a stage that binds none (pan_cmdstream.c:2029-2031): the
/// packer's default, `06a99901` and zeroes.
fn mesa_unused_sampler() -> [u8; 32] {
    let s = Sampler::default().pack().unwrap();
    assert_eq!(words(&s), [0x06a9_9901, 0, 0, 0, 0, 0, 0, 0]);
    s
}

/// RGBA8 UNORM read RGBA (pan_format.c:508: format 187, order RGBA = 0).
const FORMAT_RGBA8: u32 = 0xbb << 12;
/// `pan_translate_swizzle_4`: three bits a channel, 0..3 a channel, 5 one.
const SWIZZLE_RGBA: u32 = 1 << 3 | 2 << 6 | 3 << 9;
const SWIZZLE_RGB1: u32 = 1 << 3 | 2 << 6 | 5 << 9;

/// Mesa's texture descriptor of the case's map over our storage (module
/// comment; the lines are pan_texture.c's), at `surfaces`.
fn mesa_texture(c: &Case, surfaces: u64) -> Texture {
    Texture {
        /* :1300 */
        dimension: TextureDimension::CUBE,
        /* :1301, our RGBA8 storage */
        format: FORMAT_RGBA8,
        /* :1302-1303 */
        width: c.size as u32,
        height: c.size as u32,
        /* :1307 */
        sample_count: 1,
        /* :1308 */
        swizzle: if c.rgba { SWIZZLE_RGBA } else { SWIZZLE_RGB1 },
        /* :1310, linear storage */
        texel_interleave: false,
        /* :1315 */
        levels: c.levels as u32,
        /* :1316, :1198-1217 */
        array_size: 1,
        /* :1319 */
        surfaces,
        /* :1321-1322 */
        minimum_lod: 0.0,
        maximum_lod: (c.levels - 1) as f32,
        minimum_level: 0,
        depth: 1,
        ..Default::default()
    }
}

fn words(d: &[u8; 32]) -> [u32; 8] {
    core::array::from_fn(|i| u32::from_le_bytes(d[i * 4..i * 4 + 4].try_into().unwrap()))
}

/// A stage that reads the map: its sampler and texture are Mesa's, and its
/// planes are one a face and level, face-major, each of its level's extent.
fn check_reads(label: &str, c: &Case, s: &Stage) {
    let want = mesa_sampler(c);
    let got = Sampler::unpack(&s.sampler).unwrap_or_else(|e| panic!("{label}: the sampler does not unpack: {e:?}"));
    assert_eq!(got, want, "{label}: the sampler's fields are not Mesa's (words {:08x?})", words(&s.sampler));
    assert_eq!(s.sampler, want.pack().unwrap(), "{label}: the sampler's words");
    assert_eq!(words(&s.sampler)[0], c.word0[usize::from(!SEAMLESS)], "{label}: sampler word 0");
    assert_eq!(words(&s.sampler)[1], 0x1fff_0000, "{label}: sampler word 1 (the LOD range)");
    let (tex, planes) = s.texture.as_ref().unwrap_or_else(|| panic!("{label}: no texture in the table"));
    assert_eq!(*tex, mesa_texture(c, tex.surfaces), "{label}: the texture's fields are not Mesa's");
    let levels = c.levels as usize;
    assert_eq!(planes.len(), 6 * levels, "{label}: planes");
    for (k, p) in planes.iter().enumerate() {
        let (face, level) = (k / levels, k % levels);
        let s = (c.size >> level).max(1) as u32;
        assert_eq!(
            (p.type_, p.plane_type, p.width, p.height, p.slice_stride),
            (DescriptorType::PLANE, PlaneType::GENERIC, s, s, 0),
            "{label}: plane {k} (face {face}, level {level})"
        );
        assert!(at_gpu(p.pointer, p.size as usize).is_some(), "{label}: plane {k} points outside the buffers");
        /* a level's faces are apart and in order (pan_texture.c:418) */
        if face > 0 {
            assert!(p.pointer > planes[k - levels].pointer, "{label}: plane {k} is not after the face before");
        }
    }
}

/// A stage that reads nothing: the workaround sampler and no texture.
fn check_reads_nothing(label: &str, s: &Stage) {
    assert_eq!(
        words(&s.sampler),
        words(&mesa_unused_sampler()),
        "{label}: the sampler of the stage that samples nothing"
    );
    assert!(s.texture.is_none(), "{label}: a texture in the table of the stage that samples nothing");
}

/// Two stages' words of one map are the same but for where the planes are
/// written (`Surfaces`).
fn check_same(label: &str, a: &Stage, b: &Stage) {
    assert_eq!(words(&a.sampler), words(&b.sampler), "{label}: the two stages' sampler words differ");
    let ((ta, pa), (tb, pb)) = (a.texture.as_ref().unwrap(), b.texture.as_ref().unwrap());
    assert_eq!(*ta, Texture { surfaces: ta.surfaces, ..*tb }, "{label}: the two stages' texture words differ");
    assert_eq!(pa, pb, "{label}: the two stages' planes differ");
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

/// A cube map as dEQP makes it: every level of every face uploaded, then
/// GL_REPEAT on s and t and the filters set. Texture unit 0 is left bound
/// to it.
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
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, GL_REPEAT as GLint);
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, GL_REPEAT as GLint);
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, c.min as GLint);
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, c.mag as GLint);
    }
}

/// A triangle covering the target.
static POS2: [f32; 6] = [-1.0, -1.0, 3.0, -1.0, -1.0, 3.0];
static POS4: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
/// Directions on the +X face (t = -y, s = -z) within 0.01 of its edge with
/// +Y: the cells at a face's border of dEQP's grid.
static EDGE: [f32; 9] = [1.0, 0.995, -0.5, 1.0, 0.995, 0.5, 1.0, 0.9999, 0.0];
/// texturecubelod: the +Z face (z = 1.01) out to its edges, levels -1 .. 9.
static LOD_COORD: [f32; 9] = [-1.0, -1.0, 1.01, 1.0, -1.0, 1.01, 1.0, 1.0, 1.01];
static LOD_LEVEL: [f32; 3] = [-1.0, 4.0, 9.0];

/// An attribute: its name, components and values (three vertices).
type Attrib = (&'static std::ffi::CStr, i32, &'static [f32]);

/// Link `vert` and `frag` with attribute i named and fed by `attribs[i]`,
/// set the uniforms (`set`, with the program current) and draw. Returns the
/// draw's `glGetError` and its refusals.
fn draw(vert: &str, frag: &str, attribs: &[Attrib], set: impl FnOnce(GLuint)) -> (GLenum, u32) {
    let (v, f) = (shader(GL_VERTEX_SHADER, vert), shader(GL_FRAGMENT_SHADER, frag));
    let before = backend::draws_refused();
    // SAFETY: plain GL calls; the vertex arrays are static.
    let e = unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        for (i, (name, _, _)) in attribs.iter().enumerate() {
            glBindAttribLocation(p, i as GLuint, name.as_ptr());
        }
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        set(p);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        for i in 0..4 {
            glDisableVertexAttribArray(i);
        }
        for (i, (_, comps, values)) in attribs.iter().enumerate() {
            glVertexAttribPointer(i as GLuint, *comps, GL_FLOAT, 0, 0, values.as_ptr().cast());
            glEnableVertexAttribArray(i as GLuint);
        }
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

/* ---- the shapes ------------------------------------------------------------- */

/// `VertexCubeTextureCase`'s pair (the reference's own files).
const VCUBE_VERT: &str = include_str!("../tools/mali/testdata/pan_tex_ref/vcube.vert");
const VCUBE_FRAG: &str = include_str!("../tools/mali/testdata/pan_tex_ref/vcube.frag");

/// shaders.texture_functions.vertex.texturecubelod's pair, as
/// es2fShaderTextureFunctionTests.cpp generates it.
const LOD_VERT: &str = "attribute highp vec4 a_position;
attribute mediump vec3 a_in0;
attribute mediump float a_in1;
varying mediump vec4 v_color;
uniform lowp samplerCube u_sampler;

void main()
{
\tgl_Position = a_position;
\tv_color = textureCubeLod(u_sampler, a_in0, a_in1);
}
";
const LOD_FRAG: &str = "varying mediump vec4 v_color;

void main()
{
\tgl_FragColor = v_color;
}
";

/// The vertex cube look-up and, in the same draw, the fragment stage's
/// look-up of the same map (this test's pair; the fragment half is the
/// shape of texture.filtering.cube, which passes on the board). The sampler
/// is `mediump`: it is one uniform of the two stages, and the online
/// compiler refuses a `highp` sampler in a fragment shader (a compiler
/// refusal, which says nothing about the descriptors).
const BOTH_VERT: &str = "attribute highp vec2 a_position;
attribute highp vec3 a_texCoord;
uniform mediump samplerCube u_texture;
uniform highp float u_lod;
varying mediump vec4 v_color;
varying mediump vec3 v_dir;

void main()
{
    gl_Position = vec4(a_position, 0.0, 1.0);
    v_color = textureCubeLod(u_texture, a_texCoord, u_lod);
    v_dir = a_texCoord;
}
";
const BOTH_FRAG: &str = "precision mediump float;
uniform mediump samplerCube u_texture;
varying mediump vec4 v_color;
varying mediump vec3 v_dir;

void main()
{
    gl_FragColor = v_color * textureCube(u_texture, v_dir);
}
";

/// The fragment stage alone reads the map (texture.filtering.cube's shape).
const FS_VERT: &str = "attribute highp vec2 a_position;
attribute highp vec3 a_texCoord;
varying mediump vec3 v_dir;

void main()
{
    gl_Position = vec4(a_position, 0.0, 1.0);
    v_dir = a_texCoord;
}
";
const FS_FRAG: &str = "precision mediump float;
uniform mediump samplerCube u_texture;
varying mediump vec3 v_dir;

void main()
{
    gl_FragColor = textureCube(u_texture, v_dir);
}
";

/// The cube states of the four cases. Word 0 of Mesa's sampler, [es3, es2]:
/// type Sampler (1), the three wraps (clamp 9, repeat 8) at bits 8, 12 and
/// 16, round-to-nearest-even bit 21, seamless bit 23, normalized bit 25,
/// clamp integer array indices bit 26, magnify nearest bit 28, the mipmap
/// mode (Trilinear 3, Nearest 0) at bit 30.
const LML_LINEAR: Case = Case {
    name: "filtering.linear_mipmap_linear_linear_repeat",
    size: 32,
    levels: 6,
    rgba: false,
    min: GL_LINEAR_MIPMAP_LINEAR,
    mag: GL_LINEAR,
    word0: [0xc6a9_9901, 0xc628_8801],
};
const LML_NEAREST: Case = Case {
    name: "filtering.linear_mipmap_linear_nearest_repeat",
    mag: GL_NEAREST,
    word0: [0xd6a9_9901, 0xd628_8801],
    ..LML_LINEAR
};
const WRAP_REPEAT: Case = Case { name: "wrap.repeat_repeat", ..LML_LINEAR };
const CUBE_LOD: Case = Case {
    name: "texturecubelod",
    size: 256,
    levels: 9,
    rgba: true,
    min: GL_LINEAR_MIPMAP_NEAREST,
    mag: GL_LINEAR,
    word0: [0x06a9_9901, 0x0628_8801],
};

/// u_lod of the vertex cube's first cell: log2(32 / 28).
const VCUBE_LOD: f32 = 0.192_645_07;

fn vcube_uniforms(p: GLuint) {
    // SAFETY: plain GL calls with the program current.
    unsafe {
        glUniform1i(uniform(p, c"u_texture"), 0);
        glUniform1f(uniform(p, c"u_lod"), VCUBE_LOD);
    }
}

fn texture_uniform(p: GLuint) {
    // SAFETY: a plain GL call with the program current.
    unsafe { glUniform1i(uniform(p, c"u_texture"), 0) }
}

#[test]
fn vertex_cube_edge_descriptors_equal_mesas() {
    setup();
    let vcube: [Attrib; 2] = [(c"a_position", 2, &POS2), (c"a_texCoord", 3, &EDGE)];

    /* texture.vertex.cube.{filtering.linear_mipmap_linear_*_repeat, wrap.repeat_repeat} */
    for c in [LML_LINEAR, LML_NEAREST, WRAP_REPEAT] {
        let label = format!("vcube {}", c.name);
        cube_texture(&c);
        let (vs, fs) = stages(&label, || draw(VCUBE_VERT, VCUBE_FRAG, &vcube, vcube_uniforms));
        check_reads(&label, &c, &vs);
        check_reads_nothing(&label, &fs);

        /* the same draw's fragment stage reads the same map */
        let label = format!("vcube {} with a fragment look-up", c.name);
        let (vs2, fs2) = stages(&label, || draw(BOTH_VERT, BOTH_FRAG, &vcube, vcube_uniforms));
        check_reads(&label, &c, &vs2);
        check_reads(&label, &c, &fs2);
        check_same(&label, &vs2, &fs2);
        check_same(&label, &vs2, &vs);

        /* the fragment stage alone (the shape that passes on the board) */
        let label = format!("vcube {} in the fragment stage", c.name);
        let (vs3, fs3) = stages(&label, || draw(FS_VERT, FS_FRAG, &vcube, texture_uniform));
        check_reads_nothing(&label, &vs3);
        check_reads(&label, &c, &fs3);
        check_same(&label, &vs, &fs3);
    }

    /* shaders.texture_functions.vertex.texturecubelod */
    let c = CUBE_LOD;
    let lod: [Attrib; 3] = [(c"a_position", 4, &POS4), (c"a_in0", 3, &LOD_COORD), (c"a_in1", 1, &LOD_LEVEL)];
    cube_texture(&c);
    let (vs, fs) =
        stages(c.name, || draw(LOD_VERT, LOD_FRAG, &lod, |p| unsafe { glUniform1i(uniform(p, c"u_sampler"), 0) }));
    check_reads(c.name, &c, &vs);
    check_reads_nothing(c.name, &fs);
    let label = "texturecubelod's map in the fragment stage";
    let fs_only: [Attrib; 2] = [(c"a_position", 2, &POS2), (c"a_texCoord", 3, &LOD_COORD)];
    let (vs3, fs3) = stages(label, || draw(FS_VERT, FS_FRAG, &fs_only, texture_uniform));
    check_reads_nothing(label, &vs3);
    check_reads(label, &c, &fs3);
    check_same(label, &vs, &fs3);

    eprintln!(
        "mali vertex cube edge descriptors vs mesa ({} context): 3 vcube states and texturecubelod, vertex and fragment tables",
        if SEAMLESS { "es3" } else { "es2" }
    );
}
