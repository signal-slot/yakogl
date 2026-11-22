// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Integer-format cube textures (R8UI, R32UI, RG16I, RGBA8I, RGBA16UI,
//! RGBA32I) on the PowerVR's two cube paths: the stacked image (level-major
//! LINEAR storage, one tall 2D image; `pvr_native_cube` off) and the TPU's
//! own cube (`gl::native_cube`: a twiddled face-major copy, each face a full
//! chain on a 4 KiB boundary).
//!
//! R, RG formats are stored widened to RGBA of the same element size on the
//! PowerVR (PVR_SAMPLED_TEXFMTS) and read (r, g, 0, 1) like ES 3.0 says.
//! The host model executes no texel fetches (tests/usc_exec samples a marker
//! function, not memory), so "samples right" is pinned in three pieces that
//! meet at the bytes:
//!
//! - Placement. Six faces with distinct texels per face and level go in
//!   through glTexImage2D. In the stacked storage texel (face, level, x, y)
//!   sits at `level_off[l] + ((face * n + y) * pitch + x) * bpp`; in the
//!   native copy at `face * face_bytes + level_texel_off(p, l) * bpp +
//!   twiddle(x, y, p >> l) * bpp` (`physical_side`, `level_texel_off`),
//!   one-level draws included. Each element is read back through
//!   `TestGl::gpu_bytes` and compared exactly.
//! - Lookup. A CPU model of the TPU's NEAREST look-up (the GL face table,
//!   `s = (sc / |ma| + 1) / 2`, texel `floor(s * n)`) takes directions that
//!   hit every texel of every face of level 0 and reads the value through
//!   each layout's placement: both paths must give the value uploaded.
//! - Programs and descriptors. A `usamplerCube` / `isamplerCube` fragment
//!   shader that writes the fetch to an integer colour compiles to `smp`
//!   with `dmn` 3d on the native cube (record bit 0 set) and 2d on the
//!   stacked image, four integer channels, no float normalise; the
//!   descriptors of the native bind are TEXTYPE_CUBE with six layers and
//!   the face stride, the stacked bind a stride 2D image, in the same
//!   format word.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_int_cube
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use core::ffi::c_void;
use std::collections::BTreeMap;
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::backend::{gles_texfmt_bytes, GlesTextureBind, GLES_TEXTYPE_CUBE, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE};
use yakogl::gl::consts::*;
use yakogl::gl::context::{self, GlTexture};
use yakogl::gl::names::{self, Kind};
use yakogl::gl::native_cube::{self, level_texel_off, physical_side};
use yakogl::gl::state::{glGetError, glPixelStorei};
use yakogl::gl::texture::{glBindTexture, glGenTextures, glTexImage2D};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::online::powervr::chain::{compile_fragment, compile_vertex};
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::ShcShader;

/// The compiler keeps its output in one arena that every compile resets,
/// and the switches are process-wide: one compile at a time.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The native-cube switch set for the closure, and back at its default
/// after it, panic or not.
fn with_native_cube<T>(on: bool, f: impl FnOnce() -> T) -> T {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    struct Back;
    impl Drop for Back {
        fn drop(&mut self) {
            yakogl::shaderc::pvr_set_native_cube(yakogl::shaderc::PVR_NATIVE_CUBE_DEFAULT);
        }
    }
    let _back = Back;
    yakogl::shaderc::pvr_set_native_cube(i32::from(on));
    f()
}

/// One integer format: its GL enums and its element shape.
struct Fmt {
    name: &'static str,
    internal: u32,
    format: u32,
    type_: u32,
    channels: u32,
    /// Bytes per channel; 1, 2 or 4.
    ebytes: u32,
    signed: bool,
}

const FORMATS: &[Fmt] = &[
    Fmt {
        name: "R8UI",
        internal: GL_R8UI,
        format: GL_RED_INTEGER,
        type_: GL_UNSIGNED_BYTE,
        channels: 1,
        ebytes: 1,
        signed: false,
    },
    Fmt {
        name: "R32UI",
        internal: GL_R32UI,
        format: GL_RED_INTEGER,
        type_: GL_UNSIGNED_INT,
        channels: 1,
        ebytes: 4,
        signed: false,
    },
    Fmt {
        name: "RG16I",
        internal: GL_RG16I,
        format: GL_RG_INTEGER,
        type_: GL_SHORT,
        channels: 2,
        ebytes: 2,
        signed: true,
    },
    Fmt {
        name: "RGBA8I",
        internal: GL_RGBA8I,
        format: GL_RGBA_INTEGER,
        type_: GL_BYTE,
        channels: 4,
        ebytes: 1,
        signed: true,
    },
    Fmt {
        name: "RGBA16UI",
        internal: GL_RGBA16UI,
        format: GL_RGBA_INTEGER,
        type_: GL_UNSIGNED_SHORT,
        channels: 4,
        ebytes: 2,
        signed: false,
    },
    Fmt {
        name: "RGBA32I",
        internal: GL_RGBA32I,
        format: GL_RGBA_INTEGER,
        type_: GL_INT,
        channels: 4,
        ebytes: 4,
        signed: true,
    },
];

/// Channel `ch` of texel (x, y) of `face`, level `l`: a hash of all five,
/// cut to the element's width and sign-extended for a signed format, so no
/// two faces agree and a texel that moved reads another value.
fn value(f: &Fmt, face: u32, l: u32, x: u32, y: u32, ch: u32) -> i64 {
    let mut h = (face + 1).wrapping_mul(0x9E37_79B1)
        ^ (l + 1).wrapping_mul(0x85EB_CA6B)
        ^ (x + 1).wrapping_mul(0xC2B2_AE35)
        ^ (y + 1).wrapping_mul(0x27D4_EB2F)
        ^ (ch + 1).wrapping_mul(0x1656_67B1);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 13;
    let bits = f.ebytes * 8;
    let v = if bits == 32 { h } else { h & ((1 << bits) - 1) };
    match (f.signed, bits) {
        (false, _) => i64::from(v),
        (true, 8) => i64::from(v as u8 as i8),
        (true, 16) => i64::from(v as u16 as i16),
        (true, _) => i64::from(v as i32),
    }
}

/// An element's bytes, little endian, `ebytes` long.
fn element(f: &Fmt, v: i64) -> Vec<u8> {
    v.to_le_bytes()[..f.ebytes as usize].to_vec()
}

/// A decoded element.
fn decode(f: &Fmt, b: &[u8]) -> i64 {
    let mut raw = [0u8; 8];
    raw[..b.len()].copy_from_slice(b);
    let u = u64::from_le_bytes(raw);
    match (f.signed, f.ebytes) {
        (false, _) => u as i64,
        (true, 1) => i64::from(u as u8 as i8),
        (true, 2) => i64::from(u as u16 as i16),
        (true, _) => i64::from(u as u32 as i32),
    }
}

/// A cube of side `side`, `levels` levels of format `f`, every face and
/// level uploaded with [`value`]; the GL name.
fn upload_cube(gl: &TestGl, f: &Fmt, side: u32, levels: u32) -> GLuint {
    let mut name: GLuint = 0;
    // SAFETY: plain GL calls; each upload is its level's n x n texels.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(GL_TEXTURE_CUBE_MAP, name);
        for face in 0..6u32 {
            for l in 0..levels {
                let n = (side >> l).max(1);
                let mut px = Vec::new();
                for y in 0..n {
                    for x in 0..n {
                        for ch in 0..f.channels {
                            px.extend(element(f, value(f, face, l, x, y, ch)));
                        }
                    }
                }
                glTexImage2D(
                    GL_TEXTURE_CUBE_MAP_POSITIVE_X + face,
                    l as GLint,
                    f.internal as GLint,
                    n as GLsizei,
                    n as GLsizei,
                    0,
                    f.format,
                    f.type_,
                    px.as_ptr() as *const c_void,
                );
            }
        }
    }
    assert_eq!(gl.error(), GL_NO_ERROR, "{}: uploading a {side}x{side} cube of {levels} levels", f.name);
    name
}

/* ---- a GL context on the host: host memory (a GPU address is its CPU
 * pointer) and a stand-in back end with the PowerVR's layout that draws
 * nothing. `testutil::TestGl` is `cfg(test)` of the library and cannot be
 * linked from an integration test. ---- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
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
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
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

static STANDIN: YakoglBackend = YakoglBackend {
    name: c"pvr-int-cube".as_ptr() as *const core::ffi::c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
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
    max_instance_divisor: 1,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: None,
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
    max_samples: 0,
};

/// The one GL context of this file, current, with [`STANDIN`] as the back end.
struct TestGl {
    _serial: std::sync::MutexGuard<'static, ()>,
}

impl TestGl {
    fn new(_w: u32, _h: u32) -> TestGl {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        static ONCE: std::sync::Once = std::sync::Once::new();
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: the tables are statics; the lock serialises the GL users.
        unsafe {
            ONCE.call_once(|| {
                osal_set_ops(&OSAL);
                backend::g_backend = &STANDIN;
                assert!(!context::gl_context_create(8, 8).is_null());
            });
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            while glGetError() != GL_NO_ERROR {}
        }
        TestGl { _serial: g }
    }

    fn error(&self) -> GLenum {
        // SAFETY: a current context.
        unsafe { glGetError() }
    }

    /// The texture object of GL name `name`.
    fn texture(&self, name: GLuint) -> &GlTexture {
        let c = context::current().expect("a current context");
        let slot = names::slot_of(c, Kind::Texture, name).expect("a texture name");
        c.texture_object(slot).expect("a texture object")
    }

    /// `len` bytes at GPU address `gpu` (the host OSAL's CPU pointer).
    fn gpu_bytes(&self, gpu: u64, len: usize) -> Vec<u8> {
        // SAFETY: `gpu` is an allocation of this OSAL, at least `len` long.
        unsafe { core::slice::from_raw_parts(gpu as *const u8, len) }.to_vec()
    }
}

/// Texel index of (x, y) in a `p`-square twiddled level: y is the low bit of
/// each pair, x the next (the TPU's Morton order; the same walk as
/// `texture::twiddle_index`, which is crate-private).
fn twiddle(x: u32, y: u32, p: u32) -> u32 {
    let mut out = 0;
    for b in 0..p.max(1).trailing_zeros() {
        out |= ((y >> b) & 1) << (2 * b);
        out |= ((x >> b) & 1) << (2 * b + 1);
    }
    out
}

/// Where each path keeps texel (face, level, x, y), as a byte offset into
/// that path's memory: the stacked storage, the full-chain native copy, the
/// one-level native copy.
#[derive(Clone, Copy, Debug)]
enum Layout {
    Stacked,
    Native,
    NativeOneLevel,
}

struct Cube<'a> {
    f: &'a Fmt,
    side: u32,
    bpp: u32,
    level_off: Vec<u32>,
    pitch: Vec<u32>,
    mem: Vec<u8>,
    layout: Layout,
}

impl Cube<'_> {
    fn off(&self, face: u32, l: u32, x: u32, y: u32) -> usize {
        let n = (self.side >> l).max(1);
        let p = physical_side(self.side);
        let bpp = self.bpp;
        (match self.layout {
            Layout::Stacked => self.level_off[l as usize] + ((face * n + y) * self.pitch[l as usize] + x) * bpp,
            Layout::Native => {
                face * native_cube::face_bytes(self.side, bpp, false)
                    + (level_texel_off(p, l) + twiddle(x, y, (p >> l).max(1))) * bpp
            }
            Layout::NativeOneLevel => {
                assert_eq!(l, 0, "a one-level copy holds level 0 only");
                face * native_cube::face_bytes(self.side, bpp, true) + twiddle(x, y, p) * bpp
            }
        }) as usize
    }

    /// The channels of one texel as the GPU holds them.
    fn texel(&self, face: u32, l: u32, x: u32, y: u32) -> Vec<i64> {
        let at = self.off(face, l, x, y);
        let e = self.f.ebytes as usize;
        (0..4usize).map(|c| decode(self.f, &self.mem[at + c * e..at + (c + 1) * e])).collect()
    }
}

/// The cube's three memories read through `gpu_bytes`: the stacked storage,
/// the native copy with its chain and the one-level native copy.
fn cubes<'a>(gl: &TestGl, f: &'a Fmt, name: GLuint, side: u32, levels: u32) -> [Cube<'a>; 3] {
    let t = &gl.texture(name);
    let bpp = gles_texfmt_bytes(t.texfmt);
    /* the PowerVR samples the four-channel classes only (PVR_SAMPLED_TEXFMTS): R and RG formats are stored widened to RGBA of the same element size, the missing channels (0, 0, 1) */
    assert_eq!(bpp, 4 * f.ebytes, "{}: stored as four channels of its element size (texfmt {})", f.name, t.texfmt);
    let pitch: Vec<u32> = (0..levels).map(|l| t.level_pitch((side >> l).max(1))).collect();
    let stacked = Cube {
        f,
        side,
        bpp,
        level_off: t.level_off[..levels as usize].to_vec(),
        pitch,
        mem: gl.gpu_bytes(t.mem.gpu, t.mem.size),
        layout: Layout::Stacked,
    };
    let (g, n) = native_cube::copy(t, 0, false).unwrap_or_else(|| panic!("{}: no native copy", f.name));
    assert_eq!(n, 6 * native_cube::face_bytes(side, bpp, false), "{}: the copy's size", f.name);
    let full = Cube {
        mem: gl.gpu_bytes(g, n as usize),
        layout: Layout::Native,
        level_off: vec![],
        pitch: vec![],
        ..stacked_head(&stacked)
    };
    let (g1, n1) = native_cube::copy(t, 0, true).unwrap_or_else(|| panic!("{}: no one-level copy", f.name));
    assert_eq!(n1, 6 * native_cube::face_bytes(side, bpp, true), "{}: the one-level copy's size", f.name);
    let one = Cube {
        mem: gl.gpu_bytes(g1, n1 as usize),
        layout: Layout::NativeOneLevel,
        level_off: vec![],
        pitch: vec![],
        ..stacked_head(&stacked)
    };
    [stacked, full, one]
}

fn stacked_head<'a>(c: &Cube<'a>) -> Cube<'a> {
    Cube { f: c.f, side: c.side, bpp: c.bpp, level_off: vec![], pitch: vec![], mem: vec![], layout: c.layout }
}

/// Every texel of every face and level sits where its path puts it, exact.
#[test]
fn integer_cube_texels_sit_at_their_placement_on_every_path() {
    for f in FORMATS {
        for (side, levels) in [(4u32, 3u32), (8, 4), (1, 1)] {
            let gl = TestGl::new(4, 4);
            let name = upload_cube(&gl, f, side, levels);
            for c in cubes(&gl, f, name, side, levels) {
                let max_l = if matches!(c.layout, Layout::NativeOneLevel) { 1 } else { levels };
                for face in 0..6 {
                    for l in 0..max_l {
                        let n = (side >> l).max(1);
                        for y in 0..n {
                            for x in 0..n {
                                let want = expect(f, face, l, x, y);
                                assert_eq!(
                                    c.texel(face, l, x, y),
                                    want,
                                    "{} {side}x{side} {:?}: face {face} level {l} texel ({x}, {y}) at byte {}",
                                    f.name,
                                    c.layout,
                                    c.off(face, l, x, y)
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The face and the texel (x, y) that direction `d` hits in a face of `n`
/// texels, NEAREST (GL ES 3.0 table 3.21 and 3.8.7).
fn lookup(d: [f64; 3], n: u32) -> (u32, u32, u32) {
    let a = d.map(f64::abs);
    let (face, sc, tc, ma) = if a[0] >= a[1] && a[0] >= a[2] {
        if d[0] > 0.0 {
            (0, -d[2], -d[1], a[0])
        } else {
            (1, d[2], -d[1], a[0])
        }
    } else if a[1] >= a[2] {
        if d[1] > 0.0 {
            (2, d[0], d[2], a[1])
        } else {
            (3, d[0], -d[2], a[1])
        }
    } else if d[2] > 0.0 {
        (4, d[0], -d[1], a[2])
    } else {
        (5, -d[0], -d[1], a[2])
    };
    let texel = |c: f64| (((c / ma + 1.0) / 2.0 * f64::from(n)).floor() as i64).clamp(0, i64::from(n) - 1) as u32;
    (face, texel(sc), texel(tc))
}

/// The direction through (fx, fy) of `face`, in texels of an `n`-texel face.
fn direction(face: u32, fx: f64, fy: f64, n: u32) -> [f64; 3] {
    let s = fx / f64::from(n) * 2.0 - 1.0;
    let t = fy / f64::from(n) * 2.0 - 1.0;
    match face {
        0 => [1.0, -t, -s],
        1 => [-1.0, -t, s],
        2 => [s, 1.0, t],
        3 => [s, -1.0, -t],
        4 => [s, -t, 1.0],
        _ => [-s, -t, -1.0],
    }
}

/// Directions that hit each texel of each face at level 0 read the value
/// that was uploaded for it, through the stacked image and both native
/// copies: the face and texel the model picks are the ones aimed at.
#[test]
fn directions_on_every_face_read_the_uploaded_integer_value() {
    for f in FORMATS {
        let side = 4u32;
        let gl = TestGl::new(4, 4);
        let name = upload_cube(&gl, f, side, 3);
        for c in cubes(&gl, f, name, side, 3) {
            for face in 0..6 {
                for y in 0..side {
                    for x in 0..side {
                        /* the centre, and points off the centre inside the same texel */
                        for (dx, dy) in [(0.5, 0.5), (0.1, 0.9), (0.9, 0.1)] {
                            let d = direction(face, f64::from(x) + dx, f64::from(y) + dy, side);
                            let (hit_face, hx, hy) = lookup(d, side);
                            assert_eq!(
                                (hit_face, hx, hy),
                                (face, x, y),
                                "{} direction {d:?} aimed at face {face} ({x}, {y})",
                                f.name
                            );
                            let want = expect(f, face, 0, x, y);
                            assert_eq!(
                                c.texel(hit_face, 0, hx, hy),
                                want,
                                "{} {:?}: direction {d:?}",
                                f.name,
                                c.layout
                            );
                        }
                    }
                }
            }
        }
    }
}

/* ---- the compiled programs ---------------------------------------------- */

/// One shader through the online PowerVR chain: the record's cube bit and
/// the code image.
fn compile_with<T>(stage: Stage, src: &str, f: impl FnOnce(&ShcShader, &[u8]) -> T) -> Result<T, String> {
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, stage, text).map_err(|e| format!("front end: {}", e.msg))?;
        let rec = if stage == Stage::Fragment {
            compile_fragment(&l.module, &l.reflection, a)
        } else {
            compile_vertex(&l.module, &l.reflection, a)
        }
        .map_err(|e| format!("chain: {e:?}"))?;
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        Ok(f(&rec, &image))
    })
    .unwrap()
}

/// The `smp` fields of every sample in a program, in program order.
fn smps(image: &[u8]) -> Vec<BTreeMap<&'static str, u32>> {
    const NAMES: [&str; 5] = ["fcnorm", "dmn", "chan", "tao", "sbmode"];
    let mut off = 0usize;
    let mut out = Vec::new();
    while off < image.len() {
        let g = decode_group(image, off).expect("decodes");
        for (_, ph, r) in g.instrs() {
            if ops_for(r, ph).any(|o| o.name == "smp") {
                out.push(NAMES.iter().map(|n| (*n, r.get(n).unwrap_or(0))).collect());
            }
        }
        off += g.total;
    }
    out
}

fn enum_value(field: &str, name: &str) -> u32 {
    yakogl::shaderc::online::powervr::isa_tables::ENUMS
        .iter()
        .find(|e| e.name == field)
        .and_then(|e| e.elems.iter().find(|(_, n)| *n == name))
        .map(|(v, _)| *v)
        .unwrap_or_else(|| panic!("{field} has no value {name}"))
}

fn fragment(sampler: &str, vec: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform highp {sampler} s;\n\
         uniform vec3 d;\nout highp {vec} o;\nvoid main() {{ o = texture(s, d); }}\n"
    )
}

/// A usamplerCube / isamplerCube look-up written to an integer colour is
/// one four-channel `smp`: `dmn` 3d with the record's cube bit on the
/// native cube, `dmn` 2d with no bit on the stacked image, no float
/// normalise either way.
#[test]
fn integer_cube_programs_sample_native_and_stacked() {
    for (sampler, vec) in [("usamplerCube", "uvec4"), ("isamplerCube", "ivec4")] {
        let src = fragment(sampler, vec);
        for native in [true, false] {
            let (bit, forms) = with_native_cube(native, || {
                compile_with(Stage::Fragment, &src, |rec, image| (rec.native_cube, smps(image)))
                    .unwrap_or_else(|e| panic!("{sampler} native={native}: {e}"))
            });
            assert!(!forms.is_empty(), "{sampler} native={native}: a sample");
            for s in &forms {
                assert_eq!(s["chan"], 4, "{sampler} native={native}: four channels");
                assert_eq!(s["fcnorm"], 0, "{sampler} native={native}: integers are not normalised");
                assert_eq!(s["tao"], 0, "{sampler} native={native}: no layer address");
                assert_eq!(s["sbmode"], 0, "{sampler} native={native}: not a gather");
                let want = if native { enum_value("dmn", "3d") } else { enum_value("dmn", "2d") };
                assert_eq!(s["dmn"], want, "{sampler} native={native}: the sample's dimension");
            }
            assert_eq!(bit & 1, u8::from(native), "{sampler} native={native}: the record's cube bit");
        }
    }
}

/* ---- the descriptors ---------------------------------------------------- */

fn desc_of(t: &GlesTextureBind) -> (u64, u64, [u32; 8]) {
    let w = yakogl::pvr::backend::texture_descriptor(t);
    (
        w[0] as u64 | (w[1] as u64) << 32,
        w[2] as u64 | (w[3] as u64) << 32,
        [w[4], w[5], w[6], w[7], w[8], w[9], w[10], w[11]],
    )
}

/// The same texture bound as the TPU's cube and as the stacked image: the
/// native bind is TEXTYPE_CUBE with six layers, the copy's levels and the
/// face stride, the stacked one a stride 2D image of the tall stack, and
/// the two share the format word (everything but the texture type) and the
/// pack word of the format.
#[test]
fn integer_cube_descriptors_share_the_format_and_differ_in_type() {
    for f in FORMATS {
        let (side, levels) = (8u32, 4u32);
        let gl = TestGl::new(4, 4);
        let name = upload_cube(&gl, f, side, levels);
        let t = &gl.texture(name);
        let bpp = gles_texfmt_bytes(t.texfmt);
        let base = GlesTextureBind {
            format: t.texfmt,
            layers: 6,
            base_gpu: 0x80_0000_1000,
            width: side,
            height: side,
            stride_texels: side,
            mip_filter: 1,
            mip_levels: levels as u8,
            ..GlesTextureBind::ZERO
        };
        let native = GlesTextureBind {
            type_: GLES_TEXTYPE_CUBE_TWIDDLED_PROBE,
            bytes: 6 * native_cube::face_bytes(side, bpp, false),
            ..base
        };
        let stacked =
            GlesTextureBind { type_: GLES_TEXTYPE_CUBE, height: 6 * side, bytes: side * 6 * side * bpp, ..base };
        let (n0, n1, nm) = desc_of(&native);
        let (s0, _s1, sm) = desc_of(&stacked);
        assert_eq!(n0 & 7, 3, "{}: TEXTYPE_CUBE", f.name);
        assert_eq!(s0 & 7, 4, "{}: the stacked image is TEXTYPE_STRIDE", f.name);
        let keep = !(7 | 0x3fff << 34 | 0x3fff << 48);
        assert_eq!(n0 & keep, s0 & keep, "{}: format and swizzle words", f.name);
        assert_eq!(
            ((n0 >> 34) & 0x3fff, (n0 >> 48) & 0x3fff),
            (u64::from(side) - 1, u64::from(side) - 1),
            "{}: one face's size",
            f.name
        );
        assert_eq!(
            (n1 & 0xf, (n1 >> 4) & 0x7ff, (n1 >> 15) & 1),
            (4, 5, 1),
            "{}: four levels, six layers, mipmaps",
            f.name
        );
        assert_eq!(
            nm[0],
            native_cube::face_bytes(side, bpp, false),
            "{}: the meta layer size is one face's chain",
            f.name
        );
        assert_eq!(nm[3], sm[3], "{}: the format's pack word", f.name);
        assert_eq!(nm[7] >> 31, 0, "{}: seamless", f.name);
    }
}

/// What a look-up of texel (x, y) returns: the uploaded channels, then 0, 0, 1
/// for the ones the format does not have (ES 3.0 3.8.2).
fn expect(f: &Fmt, face: u32, l: u32, x: u32, y: u32) -> Vec<i64> {
    (0..4).map(|ch| if ch < f.channels { value(f, face, l, x, y, ch) } else { i64::from(ch == 3) }).collect()
}
