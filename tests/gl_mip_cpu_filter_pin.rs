// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GL core's CPU mipmap box filter, pinned byte for byte over odd sizes
//! and formats (plan item mip-filter-pin).
//!
//! glGenerateMipmap filters levels 1.. of a chain on the CPU in
//! `src/gl/texture.rs` (the loop after `osal::cache_invd`, the one
//! `generate_mipmap` runs when the backend answers `MipChain::None`): per
//! channel `(sum + n / 2) / n` (round half up, `n` the 2x2 box, or the
//! 1-row / 1-column pair of 2), sRGB red, green and blue averaged in linear
//! space and encoded again, the narrow L8 / A8 / LA8 texels one byte a
//! channel, half texels decoded, averaged and re-encoded toward zero, float
//! texels averaged as floats. `docs/notes/pi4-slow-terrain.md` candidate 3a
//! plans to rewrite that loop with wide loads (on the Pi 4 it reads
//! non-cacheable memory one byte at a time); this file is the byte record
//! such a rewrite must reproduce before and after.
//!
//! What is pinned: for every (format, size) case below, one FNV-1a 64 hash
//! per mip level of the chain, over that level's texel bytes (each row of
//! `w` texels at the layout's own pitch, the row padding excluded). Level 0
//! is the uploaded texels as the storage holds them, so the upload's own
//! conversions (RGB to RGBA, the narrow pack, the half and float store) are
//! pinned with the chain. The formats are the ones the box filter reads
//! (`generate_mipmap`'s `filtered` list): RGBA8, RGB8 (stored RGBA8), the
//! narrow LA8 / L8 / A8 where the backend samples narrow, sRGB8_A8, and the
//! float trio's storage classes RGBA16F, R16F and R32F. The sizes are the
//! item's: 1x1, 1xN, Nx1, 3x5, 75x37, 257x255 and 800x600 -- the tails
//! (width 1, height 1, one wide), the odd sides and a terrain-sized image.
//!
//! The texels are deterministic pseudo-random per case (an xorshift keyed by
//! the case name and size); half and float texels hold small integers, exact
//! in both formats and in every sum, so the pin does not ride on a rounding
//! of a value the rule never meant to round.
//!
//! Two readings of the same chain:
//!   - the storage bytes, read from the texture's allocation at the level
//!     offsets and pitches the record promises (`level_off`, `level_pitch_at`
//!     -- the accessors tests/es3_tex_levels.rs reads the layout with);
//!   - `ref_filter` below, an independent box filter written from the rule
//!     as `generate_mipmap` documents it, applied to the stored level 0.
//!
//! Every level's hash must equal its literal here *and* the reference's own
//! bytes must hash to the same literal: the literals are the current code's
//! bytes, and the reference shows those bytes are the documented rule. (The
//! reference reuses `gl::srgb::decode` / `encode` -- the ES 3.0.6 transfer
//! function, whose oracle is `docs/es3/srgb.md`; what it reimplements
//! independently is the box, its pair cases, its roundings and its channel
//! walk, not the curve.)
//!
//! A `report-es2` build has no sRGB or float texel format and
//! `glTexImage2D` refuses their names, so those cases only exist in an ES
//! 3.0 build; the narrow formats need the backend that samples narrow
//! (`backend::narrow_la_texels`, true on every GPU build).
//!
//! No GPU runs: the stand-in backend has no `generate_mipmap` entry, so the
//! GL core filters every case on the CPU (the path candidate 3a rewrites).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test gl_mip_cpu_filter_pin
//! # re-print the literals from the current code (into .mip-capture.txt):
//! YAKOGL_MIP_PIN_CAPTURE=1 cargo test --no-default-features --features host-test,gpu-powervr --test gl_mip_cpu_filter_pin
//! ```

#![cfg(feature = "host-test")]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::names::{self, Kind};
use yakogl::gl::srgb;
use yakogl::gl::state::{glGetError, glPixelStorei};
use yakogl::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glGenerateMipmap, glTexImage2D};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back ends that make one */

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

/// No `generate_mipmap` entry: the GL core keeps the CPU filter for every
/// texture, which is the loop this file pins.
static CPU_PATH: YakoglBackend = YakoglBackend {
    name: c"gl-mip-cpu-filter-pin".as_ptr() as *const c_char,
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
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the table is a static; the lock serialises the tests, and the
    // GL core reads the back-end table at every call.
    unsafe {
        ONCE.call_once(|| {
            osal_set_ops(&OSAL);
            backend::g_backend = &CPU_PATH;
            assert!(!context::gl_context_create(8, 8).is_null());
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        });
    }
    drain_errors();
    g
}

fn drain_errors() {
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- the cases --------------------------------------------------------- */

/// One (format, size) case: how to upload it, and what the box filter sees
/// in the storage (`comps` channels of `csize` bytes, `srgb` deciding which
/// of the four colour channels average in linear space). `src_bytes` is the
/// client texel; `comps * csize` is the stored one.
struct Case {
    pin: &'static str,
    internalformat: GLint,
    format: GLenum,
    type_: GLenum,
    src_bytes: usize,
    comps: usize,
    csize: usize,
    srgb: bool,
    narrow: bool,
}

const RGBA8: Case = Case {
    pin: "RGBA8",
    internalformat: GL_RGBA8 as GLint,
    format: GL_RGBA,
    type_: GL_UNSIGNED_BYTE,
    src_bytes: 4,
    comps: 4,
    csize: 1,
    srgb: false,
    narrow: false,
};
const RGB8: Case = Case {
    pin: "RGB8",
    internalformat: GL_RGB8 as GLint,
    format: GL_RGB,
    type_: GL_UNSIGNED_BYTE,
    src_bytes: 3,
    comps: 4,
    csize: 1,
    srgb: false,
    narrow: false,
};
const LA8: Case = Case {
    pin: "LA8",
    internalformat: GL_LUMINANCE_ALPHA as GLint,
    format: GL_LUMINANCE_ALPHA,
    type_: GL_UNSIGNED_BYTE,
    src_bytes: 2,
    comps: 2,
    csize: 1,
    srgb: false,
    narrow: true,
};
const L8: Case = Case {
    pin: "L8",
    internalformat: GL_LUMINANCE as GLint,
    format: GL_LUMINANCE,
    type_: GL_UNSIGNED_BYTE,
    src_bytes: 1,
    comps: 1,
    csize: 1,
    srgb: false,
    narrow: true,
};
const A8: Case = Case {
    pin: "A8",
    internalformat: GL_ALPHA as GLint,
    format: GL_ALPHA,
    type_: GL_UNSIGNED_BYTE,
    src_bytes: 1,
    comps: 1,
    csize: 1,
    srgb: false,
    narrow: true,
};
// The es3-only formats: report-es2 refuses their names at upload, so both
// these cases and their tests are built out of that config (see the tests).
#[cfg(not(feature = "report-es2"))]
const SRGB8_A8: Case = Case {
    pin: "SRGB8_A8",
    internalformat: GL_SRGB8_ALPHA8 as GLint,
    format: GL_RGBA,
    type_: GL_UNSIGNED_BYTE,
    src_bytes: 4,
    comps: 4,
    csize: 1,
    srgb: true,
    narrow: false,
};
#[cfg(not(feature = "report-es2"))]
const RGBA16F: Case = Case {
    pin: "RGBA16F",
    internalformat: GL_RGBA16F as GLint,
    format: GL_RGBA,
    type_: GL_HALF_FLOAT,
    src_bytes: 8,
    comps: 4,
    csize: 2,
    srgb: false,
    narrow: false,
};
#[cfg(not(feature = "report-es2"))]
const R16F: Case = Case {
    pin: "R16F",
    internalformat: GL_R16F as GLint,
    format: GL_RED,
    type_: GL_HALF_FLOAT,
    src_bytes: 2,
    comps: 1,
    csize: 2,
    srgb: false,
    narrow: false,
};
#[cfg(not(feature = "report-es2"))]
const R32F: Case = Case {
    pin: "R32F",
    internalformat: GL_R32F as GLint,
    format: GL_RED,
    type_: GL_FLOAT,
    src_bytes: 4,
    comps: 1,
    csize: 4,
    srgb: false,
    narrow: false,
};

const SIZES: [(u32, u32); 7] = [(1, 1), (1, 8), (8, 1), (3, 5), (75, 37), (257, 255), (800, 600)];
const SIZE_TAGS: [&str; 7] = ["1X1", "1X8", "8X1", "3X5", "75X37", "257X255", "800X600"];

/// The GL-core texture object behind `name`.
fn texture_object(name: GLuint) -> &'static context::GlTexture {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, name).expect("a texture name");
    c.texture_object(slot).expect("a texture object")
}

/// A deterministic generator (the `tests/common` xorshift); the case name
/// and size seed it, so every run of every build fills the same texels.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
}

fn seed(case: &Case, (w, h): (u32, u32)) -> Rng {
    let mut s = 0xcbf2_9ce4_8422_2325u64;
    for b in case.pin.as_bytes() {
        s = (s ^ *b as u64).wrapping_mul(0x100_0000_01b3);
    }
    s ^= (w as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (h as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
    if s == 0 {
        s = 1;
    }
    Rng(s)
}

/// The client texels of one case: `src_bytes` per texel, `comps` channels of
/// the channel kind it is (`csize` 1: bytes, 2: halves of integers 0..255,
/// 4: floats of integers 0..255 -- exact in every format and every sum).
fn client_texels(case: &Case, rng: &mut Rng, n: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(n * case.src_bytes);
    for _ in 0..n {
        for _ in 0..case.comps {
            match case.csize {
                1 => v.push((rng.next() & 0xff) as u8),
                2 => v.extend_from_slice(&f32_to_half((rng.next() % 256) as f32).to_le_bytes()),
                _ => v.extend_from_slice(&((rng.next() % 256) as f32).to_le_bytes()),
            }
        }
    }
    v
}

/* ---- FNV-1a ------------------------------------------------------------ */

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/* ---- reading the storage ------------------------------------------------ */

/// Level `level` of `name`: its `w` x `h` texels of `bpp` bytes, each row at
/// the layout's own pitch, the row padding excluded.
fn level_bytes(name: GLuint, level: usize, w: u32, h: u32, bpp: usize) -> Vec<u8> {
    let t = texture_object(name);
    let (off, pitch) = (t.level_off[level] as usize, t.level_pitch_at(level as u32) as usize);
    let base = t.mem.cpu as *const u8;
    assert!(!base.is_null(), "the storage has a CPU view");
    let mut v = Vec::with_capacity(w as usize * bpp * h as usize);
    for y in 0..h as usize {
        let start = off + y * pitch * bpp;
        assert!(start + w as usize * bpp <= t.mem.size, "level {level} row {y} inside the allocation");
        // SAFETY: inside the texture's host allocation (checked above).
        v.extend_from_slice(unsafe { std::slice::from_raw_parts(base.add(start), w as usize * bpp) });
    }
    v
}

/* ---- the independent reference box filter -------------------------------- */

/// IEEE half to f32: a subnormal flushes to 0 (far below 1/255), a normal
/// re-biases exactly.
fn half_to_float(h: u16) -> f32 {
    let s = ((h & 0x8000) as u32) << 16;
    let e = ((h >> 10) & 0x1f) as u32;
    let m = (h & 0x3ff) as u32;
    let bits = if e == 0 {
        0
    } else if e == 31 {
        0x7f80_0000 | (m << 13)
    } else {
        ((e + 112) << 23) | (m << 13)
    };
    f32::from_bits(s | bits)
}

/// f32 to IEEE half, round toward zero (the storage's rule): underflow 0,
/// overflow infinity. Values below 2048 with a 10-bit significand -- every
/// texel this file uploads and every average it makes -- pass exact.
fn f32_to_half(f: f32) -> u16 {
    let x = f.to_bits();
    let sign = (x >> 16) & 0x8000;
    let exp = (x >> 23) & 0xff;
    let mut mant = x & 0x7f_ffff;
    if exp == 0xff {
        return (sign | 0x7c00 | if mant != 0 { 0x200 } else { 0 }) as u16;
    }
    if exp > 142 {
        return (sign | 0x7c00) as u16;
    }
    if exp < 103 {
        return sign as u16;
    }
    if exp < 113 {
        mant |= 0x80_0000;
        return (sign | (mant >> (126 - exp))) as u16;
    }
    (sign | ((exp - 112) << 10) | (mant >> 13)) as u16
}

/// The size of the next level of `(w, h)`: a side halves while it is above
/// one and stays one at one (the pair case's other half).
fn next_size((w, h): (u32, u32)) -> (u32, u32) {
    (if w > 1 { w / 2 } else { 1 }, if h > 1 { h / 2 } else { 1 })
}

/// One level from the one before it, by the documented rule: a box of
/// `sx x sy` texels (`2 x 2`, or the pair of 2 along a side that stayed 1)
/// summed in source-row order; bytes `(sum + n / 2) / n` (round half up);
/// sRGB red, green and blue decoded, averaged and encoded again with alpha
/// linear; halves decoded, averaged and re-encoded toward zero; floats
/// averaged as floats. `src` holds `sw x sh` texels of `comps` channels of
/// `csize` bytes, tightly; so does the result (`dw x dh`).
fn ref_filter(
    src: &[u8],
    (sw, sh): (u32, u32),
    (dw, dh): (u32, u32),
    comps: usize,
    csize: usize,
    srgb: bool,
) -> Vec<u8> {
    let (sx, sy) = (if sw > 1 { 2 } else { 1 }, if sh > 1 { 2 } else { 1 });
    let n = (sx * sy) as f32;
    let bpt = comps * csize;
    let mut out = vec![0u8; (dw * dh) as usize * bpt];
    for y in 0..dh as usize {
        for x in 0..dw as usize {
            for ch in 0..comps {
                let texel = |j: usize, i: usize| (((y * sy + j) * sw as usize) + x * sx + i) * bpt + ch * csize;
                let d = (y * dw as usize + x) * bpt + ch * csize;
                if srgb && ch < 3 {
                    let mut sum = 0.0f32;
                    for j in 0..sy {
                        for i in 0..sx {
                            sum += srgb::decode(src[texel(j, i)]);
                        }
                    }
                    out[d] = srgb::encode(sum / n);
                } else if csize == 1 {
                    let mut sum: u32 = 0;
                    for j in 0..sy {
                        for i in 0..sx {
                            sum += src[texel(j, i)] as u32;
                        }
                    }
                    out[d] = ((sum + (n as u32) / 2) / (n as u32)) as u8;
                } else if csize == 2 {
                    let mut sum = 0.0f32;
                    for j in 0..sy {
                        for i in 0..sx {
                            let p = texel(j, i);
                            sum += half_to_float(u16::from_le_bytes([src[p], src[p + 1]]));
                        }
                    }
                    out[d..d + 2].copy_from_slice(&f32_to_half(sum / n).to_le_bytes());
                } else {
                    let mut sum = 0.0f32;
                    for j in 0..sy {
                        for i in 0..sx {
                            let p = texel(j, i);
                            sum += f32::from_le_bytes([src[p], src[p + 1], src[p + 2], src[p + 3]]);
                        }
                    }
                    out[d..d + 4].copy_from_slice(&(sum / n).to_le_bytes());
                }
            }
        }
    }
    out
}

/* ---- the driver ---------------------------------------------------------- */

fn capture() -> bool {
    std::env::var_os("YAKOGL_MIP_PIN_CAPTURE").is_some()
}

fn run_case(case: &Case, size_index: usize, want: &[u64]) {
    let _g = context();
    let (w, h) = SIZES[size_index];
    let mut rng = seed(case, (w, h));
    let px = client_texels(case, &mut rng, (w * h) as usize);

    let mut name = 0;
    // SAFETY: plain GL calls on the current context; `px` outlives the
    // glTexImage2D, which reads it before returning.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(GL_TEXTURE_2D, name);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            case.internalformat,
            w as GLsizei,
            h as GLsizei,
            0,
            case.format,
            case.type_,
            px.as_ptr() as *const c_void,
        );
    }
    assert_eq!(err(), GL_NO_ERROR, "{}: glTexImage2D {w}x{h}", case.pin);

    // SAFETY: plain GL call; the texture is bound to TEXTURE_2D.
    unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
    assert_eq!(err(), GL_NO_ERROR, "{}: glGenerateMipmap {w}x{h}", case.pin);

    let t = texture_object(name);
    let expect = w.max(h).max(1).ilog2() + 1;
    assert_eq!(t.chain_levels as u32, expect, "{}: the chain of {w}x{h}", case.pin);
    assert_eq!(t.bpp as usize, case.comps * case.csize, "{}: the stored texel", case.pin);
    let chain = t.chain_levels as usize;
    assert_eq!(want.len(), chain, "{} {}x{}: {} levels pinned, {} filtered", case.pin, w, h, want.len(), chain);

    // The reference: stored level 0 filtered level by level.
    let mut prev = level_bytes(name, 0, w, h, case.comps * case.csize);
    let mut ref_hashes = vec![fnv1a(&prev)];
    let mut ref_levels = vec![prev.clone()];
    let mut dims = (w, h);
    for _ in 1..chain {
        let src_dims = dims;
        dims = next_size(dims);
        let level = ref_filter(&prev, src_dims, dims, case.comps, case.csize, case.srgb);
        ref_hashes.push(fnv1a(&level));
        ref_levels.push(level.clone());
        prev = level;
    }

    let mut lines = Vec::new();
    let mut dims = (w, h);
    for l in 0..chain {
        if l > 0 {
            dims = next_size(dims);
        }
        let got = level_bytes(name, l, dims.0, dims.1, case.comps * case.csize);
        let gh = fnv1a(&got);
        if capture() {
            lines.push(format!("0x{gh:016x}"));
            continue;
        }
        assert_eq!(gh, want[l], "{} {w}x{h}: level {l} bytes moved (storage)", case.pin);
        assert_eq!(
            ref_hashes[l], want[l],
            "{} {w}x{h}: level {l} is not the documented rule (reference hash)",
            case.pin
        );
        assert_eq!(got, ref_levels[l], "{} {w}x{h}: level {l} differs from the reference byte for byte", case.pin);
    }
    if capture() {
        use std::io::Write;
        let mut f =
            std::fs::OpenOptions::new().create(true).append(true).open(".mip-capture.txt").expect("capture file");
        writeln!(f, "pub const {}_{}: [u64; {}] = [", case.pin, SIZE_TAGS[size_index], chain).unwrap();
        writeln!(f, "    {};", lines.join(", ")).unwrap();
        writeln!(f, "];").unwrap();
    }

    // SAFETY: a name this test made.
    unsafe { glDeleteTextures(1, &name) };
    assert_eq!(err(), GL_NO_ERROR);
}

fn run_fmt(case: &Case, pins: &[&[u64]]) {
    if case.narrow && !backend::narrow_la_texels() {
        eprintln!(
            "skip {}: this backend does not sample GL_LUMINANCE / GL_ALPHA / GL_LUMINANCE_ALPHA narrow",
            case.pin
        );
        return;
    }
    assert_eq!(pins.len(), SIZES.len());
    for (i, pin) in pins.iter().enumerate() {
        run_case(case, i, pin);
    }
}

/* ---- the pins -----------------------------------------------------------
 *
 * One FNV-1a 64 hash per level of the chain, level 0 first, over that
 * level's texel bytes (each row of `w` texels at the layout's own pitch,
 * the row padding excluded). Captured from `src/gl/texture.rs`'s CPU box
 * filter at the tip this file was written; the reference must land on the
 * same bytes. `YAKOGL_MIP_PIN_CAPTURE=1` re-prints them (see the header).
 */

pub const A8_1X1: [u64; 1] = [0xaf63c24c8601c05e];
pub const A8_1X8: [u64; 4] = [0x5d4612832de2fd8e, 0xb2bb27b2f84ed18f, 0x0a0de507b67ef488, 0xaf64554c8602ba27];
pub const A8_8X1: [u64; 4] = [0xf5000a6ff1c188ee, 0xbd30123e10b13506, 0x0a226f07b6908810, 0xaf63e14c8601f50b];
pub const A8_3X5: [u64; 3] = [0x1e1acafb467569bd, 0x08a20c07b549a4f0, 0xaf643e4c86029312];
pub const A8_75X37: [u64; 7] = [
    0x8ef05ac76e9f5ff3,
    0xa66883aa9776ca8d,
    0x9752d62a0aab7bf8,
    0xb80b13a36e63820f,
    0x82bf2471f20933bb,
    0x09e60807b65ddded,
    0xaf643d4c8602915f,
];
pub const A8_257X255: [u64; 9] = [
    0xe3278052e19fdebb,
    0x62eb77b0186e84e5,
    0xfd0c4402b70f728f,
    0x6fd885b94b1084c4,
    0x783df20bf8b9c75a,
    0xf45c37318f31e9d1,
    0xecfb3134e2c8553f,
    0x09e60807b65ddded,
    0xaf643d4c8602915f,
];
pub const A8_800X600: [u64; 10] = [
    0x226cfd70a466143b,
    0x57c8ffa6063e5855,
    0xd34750d15c24311c,
    0x651491471cd3e193,
    0x7aca40407635e4d4,
    0x81619f6fc18a9043,
    0x9f7927b3a81ee22c,
    0xf19d52b903014d21,
    0x92d77679eb7cd157,
    0xaf643d4c8602915f,
];
pub const L8_1X1: [u64; 1] = [0xaf643c4c86028fac];
pub const L8_1X8: [u64; 4] = [0x7dc2118717a118c2, 0xa9cf914ea07fe94e, 0x08980b07b5414eb8, 0xaf64414c8602982b];
pub const L8_8X1: [u64; 4] = [0xac8b7e929e64e052, 0x29c0d4f4a1e4708a, 0x09a16e07b6230b73, 0xaf63e64c8601fd8a];
pub const L8_3X5: [u64; 3] = [0xb04f188d10cbcaf6, 0x08fe4407b59862c5, 0xaf64114c8602469b];
pub const L8_75X37: [u64; 7] = [
    0x404be14bf2f04633,
    0x09a6ffd071aaf8c2,
    0x29c3f961c5dad9a1,
    0x658629f8ed694a61,
    0xd67e9484a37163c2,
    0x08ea4507b587bb6e,
    0xaf63f04c86020e88,
];
pub const L8_257X255: [u64; 9] = [
    0x710343ab50528e06,
    0xfaaa8c734d0f6f7e,
    0xc3248501e69538ca,
    0xde3201b983952ff6,
    0xec6426368834e23e,
    0x6c12fa8ef9e8870c,
    0x5fb8cb7d92b1d4f7,
    0x09e60807b65ddded,
    0xaf643d4c8602915f,
];
pub const L8_800X600: [u64; 10] = [
    0x22f14f4036b37362,
    0x943c73f5c1a675da,
    0xa92a319a599e03d2,
    0x9a7d00b94092bc55,
    0x78af25232200587b,
    0xf4e07d25918842ed,
    0x03a312b94003d9d5,
    0x3d1738add9f9d4c1,
    0xa6090452b278ae67,
    0xaf643c4c86028fac,
];
pub const LA8_1X1: [u64; 1] = [0x08768507b52537a0];
pub const LA8_1X8: [u64; 4] = [0xd7a8fcda704bf6be, 0x934b45adba854e03, 0x7fe0d69f54ed8569, 0x09de5907b65695e2];
pub const LA8_8X1: [u64; 4] = [0x71e2fcded8023413, 0x5692bbf3572d4d1b, 0x5d6a2f0214c656fa, 0x088dd507b5389e71];
pub const LA8_3X5: [u64; 3] = [0xe20bce2fdb579a5f, 0xec6d75f4a01b2d13, 0x08f39b07b58eef15];
pub const LA8_75X37: [u64; 7] = [
    0xa545160cd03ca0dc,
    0x9e91dc18483eef31,
    0xf7cc259c347e9a62,
    0xf53a2e23185f5645,
    0x7fe3493d32c58319,
    0xf1f94ba31b7772de,
    0x09e24907b65a6389,
];
pub const LA8_257X255: [u64; 9] = [
    0x2882ef8f8b4b3c97,
    0x983dda1fdd396980,
    0x6006658fed1decf4,
    0x65a6db189110727f,
    0xdfb5e4a0b270f578,
    0xa36723990dfa8db4,
    0xe27148453737d5d7,
    0x601ce39ff4ced3e6,
    0x09e1a307b6594977,
];
pub const LA8_800X600: [u64; 10] = [
    0xb27c3422ded9eafe,
    0xfa279a49b0f475b1,
    0xd22e7f93ff00fe04,
    0x4c53fabbc456b7e3,
    0x30fa6957e07ac716,
    0x33d229ee50ec32c3,
    0x29b464faf339ef73,
    0x241ef795a7401f8b,
    0xb996156f8af002b1,
    0x09e1a307b6594977,
];
pub const R16F_1X1: [u64; 1] = [0x09e52e07b65c6b7f];
pub const R16F_1X8: [u64; 4] = [0x1f47c61ff46c6846, 0x1d527a4a8719da41, 0x96ae8925e6da1beb, 0x0b0a2207b755e655];
pub const R16F_8X1: [u64; 4] = [0x3945a83d3c821c8c, 0xc07bd50ece57ef6c, 0x586d72ebbf437505, 0x08cb2e07b56ce3ca];
pub const R16F_3X5: [u64; 3] = [0x9cee20ed025e2059, 0x2455885d0039359b, 0x09896e07b60e7992];
pub const R16F_75X37: [u64; 7] = [
    0x5d4e2bf1f0b021ae,
    0xc89ec7ea58a5060f,
    0x65d3ea98386a3363,
    0x0b033f99fad9b6bc,
    0x24f577ad40a3eb58,
    0x049ff6da55d66d02,
    0x0ad72107b72a8b09,
];
pub const R16F_257X255: [u64; 9] = [
    0x144ee98299293448,
    0x3a398076ad4b0e52,
    0x166afb3018a46be6,
    0xe349f9aa7cfac5dc,
    0x5ee294d738d14cb3,
    0xb2355ce8565142c0,
    0x1f9d4203974a3de1,
    0x752bc65d2ed2cb2e,
    0x0aa77107b701ef37,
];
pub const R16F_800X600: [u64; 10] = [
    0xeaebc55743d4b001,
    0x03d508c911a419f0,
    0x31cebe8f47d8c52c,
    0xcac5f476bb45554c,
    0xde82a4fcd57f1e15,
    0x744e1a0af42c5003,
    0xb8aeb492a7966c66,
    0x48905ac08a284aa5,
    0xa783b3f08b5a767d,
    0x0a7b6107b6dc991c,
];
pub const R32F_1X1: [u64; 1] = [0x4b2ebc7f9c23299b];
pub const R32F_1X8: [u64; 4] = [0xfe64aaf04b4d3f1b, 0xcf9102054fd148a5, 0xf37e72878a88d9f6, 0x241e327e64f271bc];
pub const R32F_8X1: [u64; 4] = [0xa83a52ac9735eca0, 0xaf63aeaf994d6f76, 0x75c1015896b7590a, 0x0d72337dc6af3c1f];
pub const R32F_3X5: [u64; 3] = [0x0f49a5ba1d43626f, 0x9fc12d9558aa657b, 0x0d79037dc6b5093d];
pub const R32F_75X37: [u64; 7] = [
    0x3ee79cfe6155bfca,
    0xef28dccd61bd68da,
    0xbaaba86fe7dfae3c,
    0xe2458b8b2e454ecc,
    0xb3f2514f70d2f3ae,
    0x8fc1afc320b2c9b2,
    0xff4d499590380670,
];
pub const R32F_257X255: [u64; 9] = [
    0xf5152f578acc8e0f,
    0x93575b296c688c8f,
    0xaf9617af9ae11e0e,
    0x35c40fb7efd1e8af,
    0xfef2ae7d819bd1fd,
    0x1f30655b8af48367,
    0xc2e71c83ed8a445c,
    0xec522772b01ae123,
    0x4c27922256d00554,
];
pub const R32F_800X600: [u64; 10] = [
    0xb4f02df553f0398b,
    0x40493bb8aefe993c,
    0xa9d1c5d9d40fe02f,
    0x37f5629649769b8d,
    0xc46964644807404f,
    0xa3f0664314bca5c7,
    0x7de21c229fe9f706,
    0xa8e12cb89dba9441,
    0x3a52f52557c70b25,
    0x54eb570627dcd898,
];
pub const RGB8_1X1: [u64; 1] = [0x33547e409741e140];
pub const RGB8_1X8: [u64; 4] = [0xc81882db8abea49a, 0xb89acd3ac8abf0f0, 0x42373b9e3052a624, 0x1c4ffa836faeccee];
pub const RGB8_8X1: [u64; 4] = [0x8b4a719e8cb43199, 0x32cf39e0c029a294, 0xde15da2b2ebe6673, 0x3f756f97e5f992d7];
pub const RGB8_3X5: [u64; 3] = [0xf370c0317a9f5b07, 0xb45fa85d1b551c06, 0x0162b66d9ecc0f91];
pub const RGB8_75X37: [u64; 7] = [
    0xc03cce4969fcc1f4,
    0x976134d75da37a47,
    0x2cddc8c4a074e8af,
    0xf0c023bed022813f,
    0x8c8f92e4ee472576,
    0x5d0e72cebcb0b204,
    0x601f5e9ff4d027be,
];
pub const RGB8_257X255: [u64; 9] = [
    0x830d81b7241389b8,
    0x1b68edeb26e78617,
    0xbd41367dcfe2f149,
    0x4675f6cf273b48b5,
    0xf29b6e66e72639bc,
    0x0ebb815804dd14d0,
    0xb3d090292b7068ff,
    0x10d0756f943c9528,
    0x984562ad29374c5a,
];
pub const RGB8_800X600: [u64; 10] = [
    0xd376347a7d4b181b,
    0xbca4ab8bb0c7a1be,
    0x1524b09c65f58710,
    0x3c54299bb53437ee,
    0xd71e8a03c685a917,
    0x17459aaf06ec7fac,
    0x51dc823d35b29ceb,
    0x1aa59403079c6995,
    0x6ed50eb816588454,
    0x984562ad29374c5a,
];
pub const RGBA16F_1X1: [u64; 1] = [0x1322f98a6917d11d];
pub const RGBA16F_1X8: [u64; 4] = [0x7c3a3359f945def1, 0x144d0bd0002e2616, 0x0ea0b74bf058857d, 0x1c8f46df9e6f8667];
pub const RGBA16F_8X1: [u64; 4] = [0x9ad923dfc7af4dc2, 0xf64eb34d0e47d954, 0x638681aca1cd7312, 0x759abfcd45ca1aab];
pub const RGBA16F_3X5: [u64; 3] = [0x4523c8319300bbc4, 0xc9688e1dc595294a, 0x9a077f44f99ad578];
pub const RGBA16F_75X37: [u64; 7] = [
    0xc488c610ce200e78,
    0xa467955f64bdb955,
    0x15de7ddb363c7e47,
    0x811481fa9ed02570,
    0x721a5a1e331e9be3,
    0x1663197bb3dea790,
    0xa2a72e2298e01073,
];
pub const RGBA16F_257X255: [u64; 9] = [
    0xef18ee9e3fc9cea1,
    0xa87e449c31a53ad8,
    0xdde34d17caebf8c0,
    0x16df47ecefb404d0,
    0x9f079e6e223c2291,
    0x1b53ee801cb8aaab,
    0x2922354f8f39b7e9,
    0x0a3a091f05f9e535,
    0x3ded2be9e4d1f468,
];
pub const RGBA16F_800X600: [u64; 10] = [
    0x98dddba5b9870e7c,
    0xa66297dc7b2e103d,
    0xd6a282ad62fa4c9f,
    0x4d4087775165e885,
    0x7d051dd1c9e65425,
    0xe4b9cc6c27d86ead,
    0xc35a9212c412a74a,
    0x60007668c01efd21,
    0x76a37b4e750af3ac,
    0x52e5bd90b9b079bd,
];
pub const RGBA8_1X1: [u64; 1] = [0x8333c27fbd031ef0];
pub const RGBA8_1X8: [u64; 4] = [0xe0e3d196adac6717, 0x72fa6cf6756a3186, 0xd710bbd3db16dea0, 0x653cc9988c2a7928];
pub const RGBA8_8X1: [u64; 4] = [0xd1cedc2c4a55fbac, 0x307e8a9d5a997e9d, 0xc1c52c35e3c42356, 0x251960874fb09dda];
pub const RGBA8_3X5: [u64; 3] = [0xdffa92516ccd4f33, 0x1358e40a0a16e297, 0xe922543ee4df971c];
pub const RGBA8_75X37: [u64; 7] = [
    0x9696fd25c7dad42c,
    0x230ee0d632b44c33,
    0x251aa6d6c1900667,
    0x1742798936baea42,
    0xf7c815f62c5d777c,
    0x3fbb3f3d21c44bdd,
    0x120b1642f2112005,
];
pub const RGBA8_257X255: [u64; 9] = [
    0x5ddeeac73f49acab,
    0x51957e255f797175,
    0x91914136295919a1,
    0x13b90e7a6b3d8bca,
    0x5942d3da32cf62e7,
    0xb7f250d4c086a99c,
    0x6fac4c1a78396865,
    0x81f625a6686a885f,
    0x984564ad29374fc0,
];
pub const RGBA8_800X600: [u64; 10] = [
    0x7ddbf2ebf56b0dd6,
    0x7065154c9f00dbe1,
    0xdf855b724175389b,
    0xb2d09a04d3d3c3df,
    0x31c2b9707da43652,
    0x7b221aa52eb6d21a,
    0xb3b69d3500bd265d,
    0xdf5f5425121cb614,
    0x01be4231ece15acd,
    0x984565ad29375173,
];
pub const SRGB8_A8_1X1: [u64; 1] = [0x31c5ed848b2b112d];
pub const SRGB8_A8_1X8: [u64; 4] = [0x7f376b7f96e0eca1, 0xb1a52b71b2802de0, 0xd916d1ccfb77a982, 0x2541b626e9ca04c5];
pub const SRGB8_A8_8X1: [u64; 4] = [0xd50dd7336784053a, 0xafe29ee7d6ceab35, 0xd7f84de106cee2c4, 0x148979786762fe90];
pub const SRGB8_A8_3X5: [u64; 3] = [0xcbb02de3d2379812, 0x634f0aa3bf68842c, 0x024f2819b3004907];
pub const SRGB8_A8_75X37: [u64; 7] = [
    0x03cb574dda53ae2c,
    0x6a7e01633b0506c1,
    0x9eb18a706101f4fb,
    0x03d0e15b1f326a46,
    0x9e198e4f958b13a0,
    0x601d6d115086d12d,
    0xc043316cc7957d3f,
];
pub const SRGB8_A8_257X255: [u64; 9] = [
    0x03088e8bf2289a69,
    0xde48aa97d1531898,
    0x9b35e9b10591e485,
    0xd737701fd69bc0b3,
    0x2a041a88ed8ae23a,
    0x8f428351f4797cf0,
    0x479f336d465d3301,
    0x0fcf0f965bbbba95,
    0x63ef1975b0d73bc5,
];
pub const SRGB8_A8_800X600: [u64; 10] = [
    0x900d1f0d0d0857ec,
    0xdd27cac6b31dc59e,
    0xf103e57c5e993e02,
    0x52de9305760323b3,
    0x96f661e405bea0dc,
    0x8ec06aa9c4f2e785,
    0x99c96b9b8ef4158b,
    0xc61bf65bf4b504b0,
    0x8efda86c048bd702,
    0x6d045275b619e92a,
];

#[test]
fn rgba8_chain() {
    run_fmt(&RGBA8, &[&RGBA8_1X1, &RGBA8_1X8, &RGBA8_8X1, &RGBA8_3X5, &RGBA8_75X37, &RGBA8_257X255, &RGBA8_800X600]);
}

#[test]
fn rgb8_chain() {
    run_fmt(&RGB8, &[&RGB8_1X1, &RGB8_1X8, &RGB8_8X1, &RGB8_3X5, &RGB8_75X37, &RGB8_257X255, &RGB8_800X600]);
}

#[test]
fn la8_chain() {
    run_fmt(&LA8, &[&LA8_1X1, &LA8_1X8, &LA8_8X1, &LA8_3X5, &LA8_75X37, &LA8_257X255, &LA8_800X600]);
}

#[test]
fn l8_chain() {
    run_fmt(&L8, &[&L8_1X1, &L8_1X8, &L8_8X1, &L8_3X5, &L8_75X37, &L8_257X255, &L8_800X600]);
}

#[test]
fn a8_chain() {
    run_fmt(&A8, &[&A8_1X1, &A8_1X8, &A8_8X1, &A8_3X5, &A8_75X37, &A8_257X255, &A8_800X600]);
}

#[cfg(not(feature = "report-es2"))]
#[test]
fn srgb8_a8_chain() {
    run_fmt(
        &SRGB8_A8,
        &[
            &SRGB8_A8_1X1,
            &SRGB8_A8_1X8,
            &SRGB8_A8_8X1,
            &SRGB8_A8_3X5,
            &SRGB8_A8_75X37,
            &SRGB8_A8_257X255,
            &SRGB8_A8_800X600,
        ],
    );
}

#[cfg(not(feature = "report-es2"))]
#[test]
fn rgba16f_chain() {
    run_fmt(
        &RGBA16F,
        &[&RGBA16F_1X1, &RGBA16F_1X8, &RGBA16F_8X1, &RGBA16F_3X5, &RGBA16F_75X37, &RGBA16F_257X255, &RGBA16F_800X600],
    );
}

#[cfg(not(feature = "report-es2"))]
#[test]
fn r16f_chain() {
    run_fmt(&R16F, &[&R16F_1X1, &R16F_1X8, &R16F_8X1, &R16F_3X5, &R16F_75X37, &R16F_257X255, &R16F_800X600]);
}

#[cfg(not(feature = "report-es2"))]
#[test]
fn r32f_chain() {
    run_fmt(&R32F, &[&R32F_1X1, &R32F_1X8, &R32F_8X1, &R32F_3X5, &R32F_75X37, &R32F_257X255, &R32F_800X600]);
}
