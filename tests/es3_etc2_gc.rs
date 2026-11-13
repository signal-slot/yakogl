// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): a draw samples the four ETC2 names the GL core knows
//! (GL_COMPRESSED_RGB8_ETC2, RGBA8_ETC2_EAC, SRGB8_ETC2, SRGB8_ALPHA8_ETC2_EAC).
//!
//! Through the GL API on the stand-in OSAL, a `sampler2D` quad per case, and
//! the kept stream and the texture descriptor read back. Each name is
//! uploaded with glCompressedTexImage2D as one 4x4 block, and as an 8x8
//! two-level chain (four blocks, then one; TEXTURE_MAX_LEVEL 1, else the chain
//! is incomplete without 2x2 and 1x1 and the back end samples its black texel).
//! What is pinned:
//!
//! - the draw is not refused and samples the texture itself, not the back
//!   end's black texel (the descriptor's levels hold the uploaded blocks);
//! - the descriptor words of `vivante/texture.rs` against Mesa 26.2.1's
//!   `etnaviv_format.c:244-250`: RGB8_ETC2 and SRGB8_ETC2 are CONFIG0 format 0
//!   with CONFIG1's extended format EXT_NONE and alpha swizzled to ONE;
//!   RGBA8_ETC2_EAC and its sRGB twin the extended format RGBA8_ETC2_EAC; the
//!   sRGB pair differs from the linear pair in SAMP_CTRL1's SRGB bit alone
//!   (the blocks are the linear twin's, :278); all four are TILED
//!   addressing, LINEAR_STRIDE one row of blocks, SLICE a whole image of
//!   block rows, level addresses on 64 bytes with level 1 after level 0;
//! - the stored blocks are the uploaded bytes unchanged (the GC decodes them
//!   in hardware), and the blocks decode with `src/gl/etc2.rs`
//!   (`decode_rgb_image`, `decode_rgba_image`) to flat colours that differ
//!   from block to block, so a block moved to another place would show.
//!
//! What it cannot show: a picture. The host runs no GC7000 shader and renders
//! black, so the hardware's decode of the blocks, its sRGB decode, filtering
//! across block edges and the mip selection are board results (the
//! texture.wrap / texture.format dEQP rows). `etnaviv_format.c` is not
//! vendored under third_party/; its rows are cited by line as the
//! `GC_SAMPLED_TEXFMTS` doc comment (src/backend.rs) and
//! docs/es3/gc-formats.md cite them. Only the two sRGB classes are in
//! `GC_SAMPLED_TEXFMTS`; the linear pair is stored everywhere and draws
//! without a mask bit.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_etc2_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/* ---- the stand-in OSAL: host memory, and the streams the back end submits --------- */

struct Alloc {
    mem: Box<[u64]>,
    size: usize,
}

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Alloc>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
    faults: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);
/// Serialises the tests: the OSAL pointer, the back end and `g_backend` are process-global.
static SERIAL: Mutex<()> = Mutex::new(());

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    if size == 0 || size > 1 << 30 {
        return -12;
    }
    with_platform(|p| {
        let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        let at = p.bump.entry(heap_base).or_insert(0);
        let gpu = heap_base + 0x1000 + *at;
        *at += (size as u64).div_ceil(4096) * 4096;
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: the caller passes a writable descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
        p.allocs.insert(gpu, Alloc { mem, size });
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor.
    let m = unsafe { *mem };
    with_platform(|p| {
        if !m.impl_.is_null() && p.allocs.remove(&m.gpu).is_none() {
            p.faults.push(format!("gpu_free of 0x{:x}, which is no live allocation", m.gpu));
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        with_platform(|p| p.faults.push(format!("submit of a {len}-byte descriptor")));
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC {
            p.faults.push(format!("submit with magic 0x{:x}", k.magic));
            return -22;
        }
        let Some(a) = p.allocs.get(&k.stream_gpu) else {
            p.faults.push(format!("the kick's stream 0x{:x} is no allocation", k.stream_gpu));
            return -22;
        };
        // SAFETY: the allocation holds at least `stream_bytes` bytes.
        let words: &[u32] =
            unsafe { core::slice::from_raw_parts(a.mem.as_ptr() as *const u32, k.stream_bytes as usize / 4) };
        p.kicks.push(words.to_vec());
        p.fence += 1;
        p.fence
    })
}

unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.fence)
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    use yakogl::osal::{OSAL_QUERY_DRAW_STATS, OSAL_QUERY_GC_FRAME_OPEN};
    if (what == OSAL_QUERY_GC_FRAME_OPEN && out_sz == 4 && !out.is_null()) || what == OSAL_QUERY_DRAW_STATS {
        return 0;
    }
    -22
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(p_gpu_alloc),
    gpu_free: Some(p_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(p_cache),
    cache_invd: Some(p_cache),
    submit: Some(p_submit),
    wait: Some(p_wait),
    completed: Some(p_completed),
    reg_read: None,
    reg_write: None,
    query: Some(p_query),
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/// `n` bytes of the stand-in's memory at GPU address `gpu`.
fn read_gpu(gpu: u64, n: usize) -> Vec<u8> {
    with_platform(|p| {
        let (base, a) = p.allocs.range(..=gpu).next_back().unwrap_or_else(|| panic!("no allocation at 0x{gpu:x}"));
        let off = (gpu - base) as usize;
        assert!(off + n <= a.size, "0x{gpu:x}+{n} runs off the allocation at 0x{base:x} ({} bytes)", a.size);
        // SAFETY: inside the allocation (checked just above).
        unsafe { core::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(off), n) }.to_vec()
    })
}

/* ---- decoding a submitted stream ------------------------------------------------------ */

/// Every `(register, value)` a LOAD_STATE of the stream writes, in order.
fn writes(words: &[u32]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        if w & 0xf800_0000 == GC_OP_LOAD_STATE {
            let count = ((w >> 16) & 0x3ff) as usize;
            let base = (w & 0xffff) * 4;
            for k in 0..count {
                if let Some(v) = words.get(i + 1 + k) {
                    out.push((base + 4 * k as u32, *v));
                }
            }
            i += 1 + count;
            continue;
        }
        i += match w & 0xf800_0000 {
            GC_OP_DRAW_INSTANCED => 4,
            GC_OP_STALL => 2,
            _ => 1,
        };
    }
    out
}

/* ---- a fresh context, back end and OSAL per case -------------------------------------- */

fn on_gc7000(case: impl FnOnce()) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        yakogl::vivante::backend::gc_shutdown();
    }
    let faults = with_platform(|p| {
        /* leaked, not freed: addresses keep counting up (gc_npot_repeat.rs) */
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a.mem);
        }
        core::mem::take(&mut p.faults)
    });
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
}

/* ---- GL plumbing ------------------------------------------------------------------------ */

fn no_error(what: &str) {
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
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
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

const VS: &str = "#version 300 es\nin vec4 p;\nout vec2 v;\nvoid main() { v = p.xy * 0.5 + 0.5; gl_Position = p; }\n";
const FS: &str = concat!(
    "#version 300 es\nprecision mediump float;\nuniform sampler2D s;\nin vec2 v;\nout vec4 o;\n",
    "void main() { o = texture(s, v); }\n"
);

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// What one draw programmed for sampler slot 0: SAMP_CTRL1 and the descriptor.
struct Sampled {
    ctrl1: u32,
    desc: Vec<u32>,
}

impl Sampled {
    fn word(&self, off: u32) -> u32 {
        self.desc[off as usize / 4]
    }
    fn config0(&self) -> u32 {
        self.word(GC_TEXDESC_CONFIG0)
    }
    fn config1(&self) -> u32 {
        self.word(GC_TEXDESC_CONFIG1)
    }
    fn level_gpu(&self, lod: u32) -> u64 {
        u64::from(self.word(texdesc_lod_addr(lod)))
    }
}

/// One quad with the program of `FS` sampling unit 0, drawn and finished.
fn render_quad() -> Sampled {
    let kicks0 = with_platform(|p| p.kicks.len());
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    }
    no_error("draw");
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    no_error("glFinish");
    let stream = with_platform(|p| {
        assert!(p.kicks.len() > kicks0, "the quad produced no kick");
        p.kicks.last().cloned().unwrap()
    });
    let w = writes(&stream);
    let last = |reg: u32| w.iter().rev().find(|(r, _)| *r == reg).map(|(_, v)| *v);
    let must = |reg: u32, name: &str| last(reg).unwrap_or_else(|| panic!("{name} (0x{reg:x}) was never written"));
    let desc_gpu = u64::from(must(nte_descriptor_addr(0), "NTE_DESCRIPTOR_ADDR(0)"));
    assert_ne!(desc_gpu, 0, "no descriptor for slot 0");
    let desc: Vec<u32> =
        read_gpu(desc_gpu, GC_TEXDESC_BYTES).as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
    Sampled { ctrl1: must(nte_descriptor_samp_ctrl1(0), "NTE_DESCRIPTOR_SAMP_CTRL1(0)"), desc }
}

/* ---- the blocks --------------------------------------------------------------------------- */

/// One ETC2 colour block, individual mode, every texel index 0: the same
/// 4-bit base colour in both halves, codeword 0, so all sixteen texels are
/// one colour.
fn rgb_block(r: u8, g: u8, b: u8) -> [u8; 8] {
    [r << 4 | r, g << 4 | g, b << 4 | b, 0, 0, 0, 0, 0]
}

/// One EAC alpha block: base 0x80, multiplier 1, table 0, every index 0.
fn eac_block() -> [u8; 8] {
    [0x80, 0x10, 0, 0, 0, 0, 0, 0]
}

/// `n` blocks, each a different colour so one in the wrong place shows.
fn blocks(alpha: bool, n: usize) -> Vec<u8> {
    let mut v = Vec::new();
    for i in 0..n as u8 {
        if alpha {
            v.extend_from_slice(&eac_block());
        }
        v.extend_from_slice(&rgb_block(2 + 3 * i, 12 - 2 * i, 5 + i));
    }
    v
}

fn param(target: GLenum, pname: GLenum, v: GLenum) {
    // SAFETY: a plain GL call.
    unsafe { glTexParameteri(target, pname, v as GLint) };
}

/// A texture of `internal` bound to unit 0 with each `(width, height, data)`
/// of `levels` as its level i; NEAREST, mip-NEAREST with more than one level.
fn compressed_2d(internal: GLenum, levels: &[(i32, i32, &[u8])]) {
    // SAFETY: plain GL calls; each `data` holds its level's blocks.
    unsafe {
        let mut t: GLuint = 0;
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        for (i, (w, h, d)) in levels.iter().enumerate() {
            glCompressedTexImage2D(
                GL_TEXTURE_2D,
                i as GLint,
                internal,
                *w,
                *h,
                0,
                d.len() as GLsizei,
                d.as_ptr().cast(),
            );
        }
    }
    no_error("glCompressedTexImage2D");
    let min = if levels.len() > 1 { GL_NEAREST_MIPMAP_NEAREST } else { GL_NEAREST };
    /* the chain ends at the last level uploaded, so it is complete without 2x2 and 1x1 */
    param(GL_TEXTURE_2D, GL_TEXTURE_MAX_LEVEL, levels.len() as GLenum - 1);
    param(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, min);
    param(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
}

/// One name under test: its GL enum, whether its blocks carry the EAC alpha
/// half, and whether it samples through the sRGB decode.
struct Name {
    gl: GLenum,
    label: &'static str,
    alpha: bool,
    srgb: bool,
}

const NAMES: [Name; 4] = [
    Name { gl: GL_COMPRESSED_RGB8_ETC2, label: "RGB8_ETC2", alpha: false, srgb: false },
    Name { gl: GL_COMPRESSED_RGBA8_ETC2_EAC, label: "RGBA8_ETC2_EAC", alpha: true, srgb: false },
    Name { gl: GL_COMPRESSED_SRGB8_ETC2, label: "SRGB8_ETC2", alpha: false, srgb: true },
    Name { gl: GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC, label: "SRGB8_ALPHA8_ETC2_EAC", alpha: true, srgb: true },
];

/// The hardware's TEXTURE_SWIZZLE values (common_3d.xml.h:117-122).
const SW_R: u32 = 0;
const SW_G: u32 = 1;
const SW_B: u32 = 2;
const SW_A: u32 = 3;
const SW_1: u32 = 5;

/// What `etnaviv_format.c:244-250` and `etnaviv_texture_desc.c` decide for
/// `name`, against the descriptor a `w`x`h` texture of `levels` levels drew
/// with; `stored` are the uploaded blocks of each level.
fn check_descriptor(name: &Name, s: &Sampled, w: u32, h: u32, levels: u32, stored: &[&[u8]]) {
    let l = name.label;
    let block = if name.alpha { 16 } else { 8 };
    /* CONFIG0: block formats are TILED, and the RGB8 ones sit in the
     * extended format, so the CONFIG0 format field is 0 */
    assert_eq!(
        s.config0(),
        texdesc_type(GC_TEX_TYPE_2D) | texdesc_addressing_mode(GC_TEX_ADDRESSING_TILED),
        "{l}: CONFIG0"
    );
    let ext = if name.alpha { GC_TEX_FORMAT_EXT_RGBA8_ETC2_EAC } else { GC_TEX_FORMAT_EXT_NONE };
    let a = if name.alpha { SW_A } else { SW_1 };
    assert_eq!(
        s.config1(),
        texdesc_format_ext(ext)
            | texdesc_swizzle_r(SW_R)
            | texdesc_swizzle_g(SW_G)
            | texdesc_swizzle_b(SW_B)
            | texdesc_swizzle_a(a),
        "{l}: CONFIG1"
    );
    /* SAMP_CTRL1: Mesa's UNK1, plus SRGB on the sRGB pair alone */
    assert_eq!(s.ctrl1, GC_SAMP_CTRL1_UNK1 | if name.srgb { GC_SAMP_CTRL1_SRGB } else { 0 }, "{l}: SAMP_CTRL1");
    let stride = w.div_ceil(4) * block;
    assert_eq!(s.word(GC_TEXDESC_LINEAR_STRIDE), stride, "{l}: LINEAR_STRIDE is one row of blocks");
    assert_eq!(s.word(GC_TEXDESC_SLICE), stride * h.div_ceil(4), "{l}: SLICE is the image in block rows");
    assert_eq!(s.word(GC_TEXDESC_SIZE), texdesc_size_width(w) | texdesc_size_height(h), "{l}: SIZE");
    assert_eq!(s.word(GC_TEXDESC_BASELOD), texdesc_baselod_base(0) | texdesc_baselod_max(levels - 1), "{l}: BASELOD");
    let mut prev_end = 0u64;
    for (lod, bytes) in stored.iter().enumerate() {
        let at = s.level_gpu(lod as u32);
        assert_eq!(at % GC_TEXDESC_LOD_ALIGN as u64, 0, "{l}: level {lod} address 0x{at:x} is on 64 bytes");
        assert!(at >= prev_end, "{l}: level {lod} overlaps the level before");
        prev_end = at + bytes.len() as u64;
        assert_eq!(&read_gpu(at, bytes.len()), bytes, "{l}: level {lod} holds the uploaded blocks, unchanged");
    }
}

/// `data` decoded by `src/gl/etc2.rs` to the RGBA8 texels of a `w`x`h` image.
fn decode(alpha: bool, w: usize, h: usize, data: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 4];
    let ok = if alpha {
        yakogl::gl::etc2::decode_rgba_image(w, h, data, &mut out)
    } else {
        yakogl::gl::etc2::decode_rgb_image(w, h, data, false, &mut out)
    };
    assert!(ok, "src/gl/etc2.rs refused the {w}x{h} blocks");
    out
}

/// Each block of `data` decodes to one flat colour, the colours differ from
/// block to block, and an RGB name's texels are opaque.
fn check_decoded(name: &Name, w: usize, h: usize, data: &[u8]) {
    let px = decode(name.alpha, w, h, data);
    let mut seen: Vec<[u8; 4]> = Vec::new();
    for by in 0..h / 4 {
        for bx in 0..w / 4 {
            let at = |x: usize, y: usize| {
                let o = ((by * 4 + y) * w + bx * 4 + x) * 4;
                [px[o], px[o + 1], px[o + 2], px[o + 3]]
            };
            let first = at(0, 0);
            for y in 0..4 {
                for x in 0..4 {
                    assert_eq!(at(x, y), first, "{}: block ({bx},{by}) is not flat", name.label);
                }
            }
            if !name.alpha {
                assert_eq!(first[3], 255, "{}: an RGB block is opaque", name.label);
            }
            assert!(!seen.contains(&first), "{}: block ({bx},{by}) decodes like an earlier one", name.label);
            seen.push(first);
        }
    }
}

/* ---- the cases ------------------------------------------------------------------------------ */

#[test]
fn a_4x4_block_of_each_name_is_sampled() {
    for name in &NAMES {
        on_gc7000(|| {
            let data = blocks(name.alpha, 1);
            compressed_2d(name.gl, &[(4, 4, &data)]);
            let s = render_quad();
            check_descriptor(name, &s, 4, 4, 1, &[&data]);
            check_decoded(name, 4, 4, &data);
        });
    }
}

#[test]
fn an_8x8_chain_of_two_levels_is_sampled() {
    for name in &NAMES {
        on_gc7000(|| {
            let l0 = blocks(name.alpha, 4);
            let l1 = blocks(name.alpha, 1);
            compressed_2d(name.gl, &[(8, 8, &l0), (4, 4, &l1)]);
            let s = render_quad();
            check_descriptor(name, &s, 8, 8, 2, &[&l0, &l1]);
            assert!(s.level_gpu(1) >= s.level_gpu(0) + l0.len() as u64, "{}: level 1 follows level 0", name.label);
            check_decoded(name, 8, 8, &l0);
            check_decoded(name, 4, 4, &l1);
        });
    }
}

#[test]
fn the_srgb_pair_differs_from_the_linear_pair_in_the_srgb_bit_alone() {
    let mut words = Vec::new();
    for name in &NAMES {
        on_gc7000(|| {
            let data = blocks(name.alpha, 1);
            compressed_2d(name.gl, &[(4, 4, &data)]);
            let s = render_quad();
            words.push((s.config0(), s.config1(), s.ctrl1));
        });
    }
    for (lin, srgb) in [(0, 2), (1, 3)] {
        assert_eq!(words[lin].0, words[srgb].0, "CONFIG0 of {}", NAMES[srgb].label);
        assert_eq!(words[lin].1, words[srgb].1, "CONFIG1 of {}", NAMES[srgb].label);
        assert_eq!(words[lin].2 ^ words[srgb].2, GC_SAMP_CTRL1_SRGB, "SAMP_CTRL1 of {}", NAMES[srgb].label);
    }
    assert_ne!(words[0].1, words[1].1, "the EAC pair names its own extended format");
}
