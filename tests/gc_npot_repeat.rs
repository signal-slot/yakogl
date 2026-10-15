// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): what the back end emits for the dEQP-GLES2 NPOT wrap
//! cases of docs/es2-cts-failures.md section H.3, pinned register by register
//! (item gc-flake-instrument, part 2; docs/notes/gc-npot-repeat.md).
//!
//! `GL_OES_texture_npot` is listed, so dEQP expects REPEAT and
//! MIRRORED_REPEAT to work on an NPOT texture (section H.2). tests/es2_npot.rs
//! checks that the GL core calls them complete; the board's tree passes them
//! (H.4). This file checks the other half on the host: the texture descriptor
//! and the sampler registers the Vivante back end writes for the same GL
//! calls, against the register meaning of the generated rnndb headers
//! (driver/src/vivante/hw/state_3d.xml.h, common_3d.xml.h; the descriptor's own
//! word offsets are src/vivante/regs.rs, from Mesa's texdesc_3d) and
//! src/vivante/VIVANTE_NOTES.md.
//!
//! How it runs: the real GL core, the real Vivante back end (`gc_draw`,
//! `gc_flush`) and the in-process compiler for the quad's two programs. Only
//! the OSAL is a stand-in: it keeps every allocation in host memory and every
//! submitted command stream. The test decodes the stream (LOAD_STATE writes),
//! takes the descriptor `NTE_DESCRIPTOR_ADDR(0)` names and reads the texel
//! memory the descriptor's level addresses point at. So each case asserts
//!
//! - the sampler register `NTE_DESCRIPTOR_SAMP_CTRL0`, field by field (wrap
//!   S/T/W, min, mip, mag filter, and the two bits the back end always sets);
//! - every non-zero word of the 256-byte descriptor: type, format and
//!   addressing (CONFIG0), size, log size, level count (BASELOD), stride,
//!   slice and depth, and that no other word holds a bit (the HALTI5
//!   descriptor has no NPOT bit: NPOT support is the chip feature
//!   `chipMinorFeatures1_NON_POWER_OF_TWO`, common.xml.h:181, not a
//!   descriptor field, so "no stray bit" is the whole NPOT check);
//! - that the descriptor's level addresses, stride and slice reach the texels
//!   the application uploaded (the GL core's storage for 2D; the back end's
//!   4x4-tiled copy for a cube, `tiled_copy.rs`, which an NPOT cube face
//!   always takes).
//!
//! What it cannot see: the GPU. Whether the texture unit wraps at the SIZE
//! field (127), not at the padded storage (128), is the board's answer; the
//! i.MX8MP tree passes these 18 cases (H.4).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_npot_repeat
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gc_npot_repeat
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_npot_repeat -- --ignored
//! ```
//!
//! 17 tests pass. One is `#[ignore = "BUG: ..."]` and fails when run:
//! `TEXDESC_LOG_SIZE_EXT` is a linear approximation of log2, not Mesa's exact
//! value, which shows at an NPOT size like 90 (the CTS sizes are within one unit).

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

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

/* ---- register meaning, from the rnndb headers --------------------------------------
 *
 * Written out here (not taken from regs.rs) so that a wrong constant there
 * shows. Each is cited at the line that defines it.
 */

/// common_3d.xml.h:108-111 TEXTURE_WRAPMODE.
const WRAP_REPEAT: u32 = 0;
const WRAP_MIRRORED_REPEAT: u32 = 1;
const WRAP_CLAMP_TO_EDGE: u32 = 2;
/// common_3d.xml.h:99-102 TEXTURE_FILTER.
const FILTER_NONE: u32 = 0;
const FILTER_NEAREST: u32 = 1;
/// common_3d.xml.h:103-107 TEXTURE_TYPE.
const TYPE_2D: u32 = 2;
const TYPE_CUBE_MAP: u32 = 5;
/// common_3d.xml.h:131-132 TEXTURE_ADDRESSING_MODE.
const ADDRESSING_TILED: u32 = 0;
const ADDRESSING_LINEAR: u32 = 3;

/// state_3d.xml.h:1953-1970 VIVS_NTE_DESCRIPTOR_SAMP_CTRL0: UWRAP bits 0-2,
/// VWRAP 3-5, WWRAP 6-8, MIN 9-10, MIP 11-12, MAG 13-14; UNK21 (bit 21) and
/// INT_FILTER (bit 23) are the two bits Mesa sets for a UNORM 2D/cube texture.
struct SampCtrl0 {
    u: u32,
    v: u32,
    w: u32,
    min: u32,
    mip: u32,
    mag: u32,
    rest: u32,
}

fn samp_ctrl0_fields(r: u32) -> SampCtrl0 {
    SampCtrl0 {
        u: r & 0x7,
        v: (r >> 3) & 0x7,
        w: (r >> 6) & 0x7,
        min: (r >> 9) & 0x3,
        mip: (r >> 11) & 0x3,
        mag: (r >> 13) & 0x3,
        rest: r & !0x7fff,
    }
}

/// UNK21 | INT_FILTER (state_3d.xml.h:1979, :1981).
const CTRL0_ALWAYS: u32 = 0x0020_0000 | 0x0080_0000;

/// A8B8G8R8: R,G,B,A in memory (src/vivante/regs.rs, `GC_TEX_FORMAT_A8B8G8R8`).
const FORMAT_RGBA8_IN_MEMORY: u32 = 0x09;

/// Descriptor word offsets (src/vivante/regs.rs `GC_TEXDESC_*`).
const D_CONFIG0: usize = 0x40 / 4;
const D_SIZE: usize = 0x44 / 4;
const D_STRIDE: usize = 0x48 / 4;
const D_CONFIG1: usize = 0x4c / 4;
const D_BASELOD: usize = 0x68 / 4;
const D_CONFIG2: usize = 0x6c / 4;
const D_LOG_SIZE_EXT: usize = 0x74 / 4;
const D_VOLUME: usize = 0x78 / 4;
const D_SLICE: usize = 0x7c / 4;
const D_3D_CONFIG: usize = 0x84 / 4;

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
    lock: None,
    unlock: None,
    thread_id: None,
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

fn texel_at(gpu: u64) -> [u8; 4] {
    read_gpu(gpu, 4).try_into().unwrap()
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

/// What one draw of the quad programmed for sampler slot 0.
struct Sampled {
    ctrl0: u32,
    ctrl1: u32,
    tx_ctrl: u32,
    lod_minmax: u32,
    lod_bias: u32,
    anisotropy: u32,
    desc: Vec<u32>,
}

impl Sampled {
    fn lod_addr(&self, l: usize) -> u64 {
        u64::from(self.desc[l])
    }
    fn stride(&self) -> u64 {
        u64::from(self.desc[D_STRIDE])
    }
    fn slice(&self) -> u64 {
        u64::from(self.desc[D_SLICE])
    }
}

/* ---- a fresh context, back end and OSAL per case -------------------------------------- */

/// Run `case` with the Vivante back end up on the stand-in OSAL and a fresh
/// 64x64 context (so the default texture objects are pristine, as at the
/// start of a CTS case group).
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
        /* The memory is leaked, not freed, and the addresses keep counting up:
         * a static of the library that still names an earlier case's block
         * (a cache keyed by GPU address) must find memory, not a hole or a
         * different case's data. tests/es2_npot.rs leaks its blocks the same way. */
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

/* ---- GL plumbing (the CTS's calls, as in tests/es2_npot.rs) ---------------------------- */

fn no_error(what: &str) {
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}, the CTS expects GL_NO_ERROR (expectError)");
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

fn program(vs: &str, fs: &str) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        p
    }
}

const VS: &str = "attribute vec4 p;\nvarying vec2 v;\nvoid main() { v = p.xy * 0.5 + 0.5; gl_Position = p; }\n";
const FS_2D: &str =
    "precision mediump float;\nuniform sampler2D s;\nvarying vec2 v;\nvoid main() { gl_FragColor = texture2D(s, v); }\n";
const FS_CUBE: &str = "precision mediump float;\nuniform samplerCube s;\nvarying vec2 v;\nvoid main() { gl_FragColor = textureCube(s, vec3(v * 2.0 - 1.0, 1.0)); }\n";

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// The CTS's TextureRenderer::renderQuad (viewport, the program of the
/// texture type, one quad), then the kick: returns what the back end wrote
/// for sampler slot 0 in the stream that carried the draw.
fn render_quad(cube: bool) -> Sampled {
    let kicks0 = with_platform(|p| p.kicks.len());
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        let p = program(VS, if cube { FS_CUBE } else { FS_2D });
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    }
    no_error("renderQuad");
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
    let desc_bytes = read_gpu(desc_gpu, GC_TEXDESC_BYTES);
    let desc: Vec<u32> = desc_bytes.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
    Sampled {
        ctrl0: must(nte_descriptor_samp_ctrl0(0), "NTE_DESCRIPTOR_SAMP_CTRL0(0)"),
        ctrl1: must(nte_descriptor_samp_ctrl1(0), "NTE_DESCRIPTOR_SAMP_CTRL1(0)"),
        tx_ctrl: must(nte_descriptor_tx_ctrl(0), "NTE_DESCRIPTOR_TX_CTRL(0)"),
        lod_minmax: must(nte_descriptor_samp_lod_minmax(0), "NTE_DESCRIPTOR_SAMP_LOD_MINMAX(0)"),
        lod_bias: must(nte_descriptor_samp_lod_bias(0), "NTE_DESCRIPTOR_SAMP_LOD_BIAS(0)"),
        anisotropy: must(nte_descriptor_samp_anisotropy(0), "NTE_DESCRIPTOR_SAMP_ANISOTROPY(0)"),
        desc,
    }
}

fn tex_param(target: GLenum, pname: GLenum, v: GLenum) {
    // SAFETY: a plain GL call.
    unsafe { glTexParameteri(target, pname, v as GLint) };
}

fn tex_image(target: GLenum, level: i32, w: i32, h: i32, texels: &[u8]) {
    assert!(texels.len() >= (w * h * 4) as usize);
    // SAFETY: `texels` holds the w x h image.
    unsafe {
        glTexImage2D(target, level, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels.as_ptr().cast());
    }
}

const FACES: [GLenum; 6] = [
    GL_TEXTURE_CUBE_MAP_POSITIVE_X,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
];

/// Texel (x, y) of layer `layer` at `level`: every position and layer reads
/// differently, so a wrong stride, slice or level address shows as a wrong texel.
fn texel(x: u32, y: u32, layer: u32, level: u32) -> [u8; 4] {
    [x as u8, y as u8, (0x10 + layer * 0x20 + level) as u8, 0xff]
}

fn image(w: u32, h: u32, layer: u32, level: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            v.extend_from_slice(&texel(x, y, layer, level));
        }
    }
    v
}

/* ---- the expectations ---------------------------------------------------------------- */

/// The GL wrap enum as the rnndb TEXTURE_WRAPMODE value.
fn hw_wrap(w: GLenum) -> u32 {
    match w {
        GL_REPEAT => WRAP_REPEAT,
        GL_MIRRORED_REPEAT => WRAP_MIRRORED_REPEAT,
        _ => WRAP_CLAMP_TO_EDGE,
    }
}

fn wrap_name(w: u32) -> &'static str {
    ["REPEAT", "MIRRORED_REPEAT", "CLAMP_TO_EDGE", "CLAMP_TO_BORDER"][w as usize & 3]
}

/// log2(v) in 8.8 fixed point as `texture.rs` `log2_fixp88` writes it: the
/// integer part and, for the fraction, the linear position between 2^k and
/// 2^(k+1).
fn backend_log2_fixp88(v: u32) -> u32 {
    let k = 31 - v.leading_zeros();
    (k << 8) | ((((v - (1 << k)) << 8) / (1 << k)) & 0xff)
}

/// What Mesa writes (`etna_log2_fixp88`, etnaviv_util.h:57, reached from
/// etnaviv_texture_desc.c:200-201; `S_FIXED` is a truncating cast):
/// `(int32_t)(log2f(v) * 256)`.
fn mesa_log2_fixp88(v: u32) -> u32 {
    ((v as f32).log2() * 256.0) as u32
}

struct Expect {
    /// The wrap the sampler register must hold, as GL enums (S, T).
    wrap: (GLenum, GLenum),
    /// MIN filter field; MIP filter field; MAG filter field (TEXTURE_FILTER).
    filters: (u32, u32, u32),
    /// The base level's size.
    size: (u32, u32),
    /// BASELOD's MAXLOD: levels - 1.
    max_lod: u32,
    cube: bool,
}

fn assert_sampler(shape: &str, s: &Sampled, e: &Expect) {
    let c = samp_ctrl0_fields(s.ctrl0);
    let (ws, wt) = (hw_wrap(e.wrap.0), hw_wrap(e.wrap.1));
    assert_eq!(
        c.u,
        ws,
        "[{shape}] SAMP_CTRL0.UWRAP (bits 0-2) is {}, GL_TEXTURE_WRAP_S asked for {}",
        wrap_name(c.u),
        wrap_name(ws)
    );
    assert_eq!(
        c.v,
        wt,
        "[{shape}] SAMP_CTRL0.VWRAP (bits 3-5) is {}, GL_TEXTURE_WRAP_T asked for {}",
        wrap_name(c.v),
        wrap_name(wt)
    );
    /* WWRAP follows UWRAP in the back end (texture.rs `samp_wwrap(uwrap)`); a 2D or
     * cube lookup has no third coordinate, so it is not a GL-visible state */
    assert_eq!(c.w, ws, "[{shape}] SAMP_CTRL0.WWRAP (bits 6-8) follows UWRAP");
    assert_eq!((c.min, c.mip, c.mag), e.filters, "[{shape}] SAMP_CTRL0 MIN (bits 9-10), MIP (11-12), MAG (13-14)");
    assert_eq!(
        c.rest, CTRL0_ALWAYS,
        "[{shape}] SAMP_CTRL0: only UNK21 (bit 21) and INT_FILTER (bit 23) besides the fields"
    );
    assert_eq!(s.ctrl1, 0x2, "[{shape}] SAMP_CTRL1 is Mesa's constant UNK1, no SRGB");
    assert_eq!(s.tx_ctrl, 0, "[{shape}] TX_CTRL: no tile status");
    assert_eq!(s.lod_minmax, 0x0fff, "[{shape}] SAMP_LOD_MINMAX: MIN 0, MAX 0xfff (GL's default 1000 saturated)");
    assert_eq!((s.lod_bias, s.anisotropy), (0, 0), "[{shape}] SAMP_LOD_BIAS, SAMP_ANISOTROPY");

    let d = &s.desc;
    let (w, h) = e.size;
    let hw_type = if e.cube { TYPE_CUBE_MAP } else { TYPE_2D };
    /* A cube face that is not a multiple of 4 wide is sampled from the back
     * end's 4x4-tiled copy (tiled_copy.rs `wanted`: the linear face step
     * stride * h is not the tile-padded align4(w) * align4(h)); a plain 2D
     * texture is sampled where the GL core stored it. */
    let addressing = if e.cube { ADDRESSING_TILED } else { ADDRESSING_LINEAR };
    assert_eq!(
        d[D_CONFIG0],
        hw_type | (FORMAT_RGBA8_IN_MEMORY << 13) | (addressing << 20),
        "[{shape}] TEXDESC_CONFIG0: TYPE (bits 0-2) {hw_type}, FORMAT (bits 13-17), ADDRESSING_MODE (bits 20-21) {addressing}"
    );
    assert_eq!(
        d[D_SIZE],
        w | (h << 16),
        "[{shape}] TEXDESC_SIZE: the texture's own {w}x{h}, not the padded storage the REPEAT wraps at"
    );
    assert_eq!(
        d[D_LOG_SIZE_EXT],
        backend_log2_fixp88(w) | (backend_log2_fixp88(h) << 16),
        "[{shape}] TEXDESC_LOG_SIZE_EXT (bits 0-15 width, 16-31 height): log2 of {w} and {h} in 8.8, as the back end computes it"
    );
    /* Mesa's value is the exact log2. At every size the CTS uses (127, 128,
     * 255) the back end is within one unit (1/256 octave) of it; elsewhere it
     * is not: see `bug_log_size_ext_is_a_linear_approximation_of_log2`. */
    for (got, size) in [(d[D_LOG_SIZE_EXT] & 0xffff, w), (d[D_LOG_SIZE_EXT] >> 16, h)] {
        assert!(
            got.abs_diff(mesa_log2_fixp88(size)) <= 1,
            "[{shape}] TEXDESC_LOG_SIZE_EXT of {size}: 0x{got:x}, Mesa writes 0x{:x}",
            mesa_log2_fixp88(size)
        );
    }
    assert_eq!(
        d[D_BASELOD],
        e.max_lod << 8,
        "[{shape}] TEXDESC_BASELOD: BASELOD (bits 0-3) 0, MAXLOD (bits 8-11) {} (the level count - 1)",
        e.max_lod
    );
    assert_eq!(d[D_CONFIG2], 0x0003_0000, "[{shape}] TEXDESC_CONFIG2 is the blob's constant");
    assert_eq!(
        d[D_CONFIG1],
        (1 << 12) | (2 << 16) | (3 << 20),
        "[{shape}] TEXDESC_CONFIG1: FORMAT_EXT 0, swizzle R,G,B,A (bits 8-23), no TEXTURE_ARRAY, HALIGN 0"
    );
    assert_eq!(d[D_VOLUME], 0, "[{shape}] TEXDESC_VOLUME: log2 of depth 1");
    assert_eq!(d[D_3D_CONFIG], 1, "[{shape}] TEXDESC_3D_CONFIG.DEPTH (bits 0-13): a cube's six faces are not a depth");
    /* the stride is a whole number of texels and covers the row */
    assert!(
        s.stride() >= u64::from(w) * 4 && s.stride().is_multiple_of(4),
        "[{shape}] TEXDESC_LINEAR_STRIDE {} covers {w} texels",
        s.stride()
    );

    /* no other word of the descriptor holds a bit: no NPOT field exists on this
     * descriptor, so a stray bit would be a different feature's */
    let named =
        [D_CONFIG0, D_SIZE, D_STRIDE, D_CONFIG1, D_BASELOD, D_CONFIG2, D_LOG_SIZE_EXT, D_VOLUME, D_SLICE, D_3D_CONFIG];
    let levels = e.max_lod as usize + 1;
    for (i, v) in d.iter().enumerate() {
        if !named.contains(&i) && i >= levels {
            assert_eq!(*v, 0, "[{shape}] descriptor word {i} (offset 0x{:x}) holds 0x{v:08x}, expected 0", i * 4);
        }
    }
    for l in 0..levels {
        assert_eq!(s.lod_addr(l) % 64, 0, "[{shape}] LOD_ADDR({l}) is on a 64-byte boundary (GC_TEXDESC_LOD_ALIGN)");
    }
}

/// Every texel of a linear 2D level reads back where the descriptor says.
fn assert_linear_level(shape: &str, s: &Sampled, level: usize, w: u32, h: u32, layer: u32) {
    for y in 0..h {
        for x in 0..w {
            let at = s.lod_addr(level) + u64::from(y) * s.stride() + u64::from(x) * 4;
            assert_eq!(
                texel_at(at),
                texel(x, y, layer, level as u32),
                "[{shape}] level {level} texel ({x},{y}) at LOD_ADDR + y * LINEAR_STRIDE + 4x"
            );
        }
    }
}

/// Every texel of every face of a tiled cube reads back where the descriptor
/// and the 4x4-tile layout (tiled_copy.rs `tiled_offset`) say.
fn assert_tiled_cube(shape: &str, s: &Sampled, size: u32) {
    let tw = size.next_multiple_of(4);
    assert_eq!(s.stride(), u64::from(tw) * 4, "[{shape}] TEXDESC_LINEAR_STRIDE of a tiled copy: align4({size}) texels");
    assert_eq!(
        s.slice(),
        u64::from(tw) * u64::from(tw) * 4,
        "[{shape}] TEXDESC_SLICE: the step between faces is align4(w) * align4(h) texels"
    );
    for f in 0..6u32 {
        for y in 0..size {
            for x in 0..size {
                let tile = u64::from(y / 4) * u64::from(tw / 4) + u64::from(x / 4);
                let within = u64::from((y % 4) * 4 + x % 4);
                let at = s.lod_addr(0) + u64::from(f) * s.slice() + (tile * 16 + within) * 4;
                assert_eq!(texel_at(at), texel(x, y, f, 0), "[{shape}] face {f} texel ({x},{y}) in the tiled copy");
            }
        }
    }
}

/* ---- Incomplete2DWrapModeCase: texture.completeness.2d.* ------------------------------- */

/// 127x127, one level, NEAREST, the wrap pair under test (constructor order
/// of the CTS: T first, then S). The texture is complete, so the descriptor
/// is the texture's own.
fn wrap_2d(wrap_t: GLenum, wrap_s: GLenum) {
    on_gc7000(|| {
        let mut t: GLuint = 0;
        // SAFETY: one name; plain GL calls.
        unsafe {
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
        }
        tex_param(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, wrap_s);
        tex_param(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, wrap_t);
        tex_param(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
        tex_param(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
        tex_image(GL_TEXTURE_2D, 0, 127, 127, &image(127, 127, 0, 0));
        no_error("set texturing state");
        let s = render_quad(false);
        let e = Expect {
            wrap: (wrap_s, wrap_t),
            filters: (FILTER_NEAREST, FILTER_NONE, FILTER_NEAREST),
            size: (127, 127),
            max_lod: 0,
            cube: false,
        };
        assert_sampler("2d 127x127", &s, &e);
        assert_linear_level("2d 127x127", &s, 0, 127, 127, 0);
    });
}

#[test]
fn texture_completeness_2d_npot_t_repeat() {
    wrap_2d(GL_REPEAT, GL_CLAMP_TO_EDGE);
}

#[test]
fn texture_completeness_2d_npot_s_repeat() {
    wrap_2d(GL_CLAMP_TO_EDGE, GL_REPEAT);
}

#[test]
fn texture_completeness_2d_npot_all_repeat() {
    wrap_2d(GL_REPEAT, GL_REPEAT);
}

#[test]
fn texture_completeness_2d_npot_mirrored_repeat() {
    wrap_2d(GL_MIRRORED_REPEAT, GL_MIRRORED_REPEAT);
}

/// 127x128 and 128x127, REPEAT on both axes, NEAREST: the NPOT side is
/// wrapped at its own size, the power-of-two side at 128.
fn repeat_one_side(w: u32, h: u32) {
    on_gc7000(|| {
        let mut t: GLuint = 0;
        // SAFETY: one name; plain GL calls.
        unsafe {
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
        }
        tex_param(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_REPEAT);
        tex_param(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT);
        tex_param(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
        tex_param(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
        tex_image(GL_TEXTURE_2D, 0, w as i32, h as i32, &image(w, h, 0, 0));
        no_error("set texturing state");
        let s = render_quad(false);
        let e = Expect {
            wrap: (GL_REPEAT, GL_REPEAT),
            filters: (FILTER_NEAREST, FILTER_NONE, FILTER_NEAREST),
            size: (w, h),
            max_lod: 0,
            cube: false,
        };
        assert_sampler(&format!("2d {w}x{h}"), &s, &e);
        assert_linear_level(&format!("2d {w}x{h}"), &s, 0, w, h, 0);
    });
}

#[test]
fn texture_completeness_2d_repeat_width_npot() {
    repeat_one_side(127, 128);
}

#[test]
fn texture_completeness_2d_repeat_height_npot() {
    repeat_one_side(128, 127);
}

/// Incomplete2DSizeCase with 255x255, REPEAT / REPEAT, NEAREST_MIPMAP_NEAREST
/// and every level the right size: the one wrap case that samples a
/// mipmapped NPOT chain (255, 127, 63, 31, 15, 7, 3, 1).
#[test]
fn texture_completeness_2d_npot_size() {
    on_gc7000(|| {
        let mut t: GLuint = 0;
        // SAFETY: one name; plain GL calls.
        unsafe {
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
        }
        tex_param(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_REPEAT);
        tex_param(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT);
        tex_param(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST);
        tex_param(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
        no_error("set texturing state");
        let sizes = [255u32, 127, 63, 31, 15, 7, 3, 1];
        for (l, w) in sizes.iter().enumerate() {
            tex_image(GL_TEXTURE_2D, l as i32, *w as i32, *w as i32, &image(*w, *w, 0, l as u32));
            no_error(&format!("glTexImage2D(level {l}, {w}x{w})"));
        }
        let s = render_quad(false);
        let e = Expect {
            wrap: (GL_REPEAT, GL_REPEAT),
            filters: (FILTER_NEAREST, FILTER_NEAREST, FILTER_NEAREST),
            size: (255, 255),
            max_lod: 7,
            cube: false,
        };
        assert_sampler("2d 255x255 mipmapped", &s, &e);
        for (l, w) in sizes.iter().enumerate() {
            assert_linear_level("2d 255x255 mipmapped", &s, l, *w, *w, 0);
        }
        /* a level starts where the one above it ends: no overlap, no gap beyond the 64-byte alignment */
        for l in 1..sizes.len() {
            let above = s.lod_addr(l - 1) + u64::from(sizes[l - 1]) * s.stride();
            assert!(
                s.lod_addr(l) >= above && s.lod_addr(l) < above + 64,
                "level {l} at 0x{:x} should follow level {} (ends 0x{above:x}) at the next 64-byte boundary",
                s.lod_addr(l),
                l - 1
            );
        }
    });
}

/* ---- IncompleteCubeWrapModeCase: texture.completeness.cube.* --------------------------- */

/// The CTS binds its new texture to GL_TEXTURE_2D and sets the parameters and
/// the six 127x127 faces on the GL_TEXTURE_CUBE_MAP binding: the default cube
/// texture. NEAREST, one level.
fn wrap_cube(wrap_t: GLenum, wrap_s: GLenum) {
    on_gc7000(|| {
        let mut t: GLuint = 0;
        // SAFETY: one name; plain GL calls.
        unsafe {
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
        }
        tex_param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, wrap_s);
        tex_param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, wrap_t);
        tex_param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
        tex_param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
        for (f, target) in FACES.iter().enumerate() {
            tex_image(*target, 0, 127, 127, &image(127, 127, f as u32, 0));
        }
        no_error("set texturing state");
        let s = render_quad(true);
        let e = Expect {
            wrap: (wrap_s, wrap_t),
            filters: (FILTER_NEAREST, FILTER_NONE, FILTER_NEAREST),
            size: (127, 127),
            max_lod: 0,
            cube: true,
        };
        assert_sampler("cube 127x127", &s, &e);
        assert_tiled_cube("cube 127x127", &s, 127);
    });
}

#[test]
fn texture_completeness_cube_npot_t_repeat() {
    wrap_cube(GL_REPEAT, GL_CLAMP_TO_EDGE);
}

#[test]
fn texture_completeness_cube_npot_s_repeat() {
    wrap_cube(GL_CLAMP_TO_EDGE, GL_REPEAT);
}

#[test]
fn texture_completeness_cube_npot_all_repeat() {
    wrap_cube(GL_REPEAT, GL_REPEAT);
}

#[test]
fn texture_completeness_cube_npot_mirrored_repeat() {
    wrap_cube(GL_MIRRORED_REPEAT, GL_MIRRORED_REPEAT);
}

/* ---- the "expect black" NPOT size cases ------------------------------------------------ */

/// An incomplete texture samples (0,0,0,1) (ES 2.0 section 3.7.10). The back
/// end answers with its own 1x1 opaque black texel, clamped, whatever wrap
/// the application asked for (texture.rs `emit_slot`). Checked for the seven
/// NPOT size cases that expect black, so a REPEAT request on a texture the
/// GL core calls incomplete does not reach the sampler.
fn assert_black_texel(shape: &str, s: &Sampled, cube_sampler: bool) {
    let c = samp_ctrl0_fields(s.ctrl0);
    assert_eq!(
        (c.u, c.v),
        (WRAP_CLAMP_TO_EDGE, WRAP_CLAMP_TO_EDGE),
        "[{shape}] an incomplete texture's stand-in is clamped: SAMP_CTRL0.UWRAP/VWRAP"
    );
    assert_eq!(c.mip, FILTER_NONE, "[{shape}] SAMP_CTRL0.MIP of the 1x1 stand-in");
    assert_eq!(s.desc[D_SIZE], 1 | (1 << 16), "[{shape}] TEXDESC_SIZE of the stand-in");
    assert_eq!(s.desc[D_BASELOD], 0, "[{shape}] TEXDESC_BASELOD: one level");
    assert_eq!(s.desc[D_STRIDE], 4, "[{shape}] TEXDESC_LINEAR_STRIDE of one texel");
    assert_eq!(texel_at(s.lod_addr(0)), [0, 0, 0, 0xff], "[{shape}] the texel LOD_ADDR(0) names is opaque black");
    /* The stand-in is a 2D descriptor even for a samplerCube program: GL core
     * and back end have no cube black texel. The i.MX8MP tree passes the four
     * cube cases (H.4), so the sampler type of the program decides. Recorded
     * here, not asserted either way beyond "2D". */
    assert_eq!(
        s.desc[D_CONFIG0] & 0x7,
        TYPE_2D,
        "[{shape}] TEXDESC_CONFIG0.TYPE of the stand-in (cube sampler: {cube_sampler})"
    );
}

fn size_2d_black(size: i32, bad_size: i32, bad_level: i32) {
    on_gc7000(|| {
        let mut t: GLuint = 0;
        // SAFETY: one name; plain GL calls.
        unsafe {
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
        }
        tex_param(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_REPEAT);
        tex_param(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT);
        tex_param(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST);
        tex_param(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
        let levels = 1 + (31 - (size as u32).leading_zeros()) as i32;
        for l in 0..levels {
            let w = if l == bad_level { bad_size } else { (size >> l).max(1) };
            tex_image(GL_TEXTURE_2D, l, w, w, &image(w as u32, w as u32, 0, l as u32));
            no_error(&format!("glTexImage2D(level {l}, {w}x{w})"));
        }
        let s = render_quad(false);
        assert_black_texel("2d size case", &s, false);
    });
}

#[test]
fn texture_completeness_2d_npot_size_level_0() {
    size_2d_black(256, 255, 0);
}

#[test]
fn texture_completeness_2d_npot_size_level_1() {
    size_2d_black(256, 127, 1);
}

fn size_cube_black(bad_size: i32, bad_level: i32, face: Option<usize>) {
    on_gc7000(|| {
        let mut t: GLuint = 0;
        // SAFETY: one name; plain GL calls.
        unsafe {
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_CUBE_MAP, t);
        }
        tex_param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, GL_REPEAT);
        tex_param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, GL_REPEAT);
        tex_param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST);
        tex_param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
        for l in 0..7 {
            for (f, target) in FACES.iter().enumerate() {
                let mut w = (64 >> l).max(1);
                if l == bad_level && face.is_none_or(|b| b == f) {
                    w = bad_size;
                }
                tex_image(*target, l, w, w, &image(w as u32, w as u32, f as u32, l as u32));
                no_error(&format!("glTexImage2D(face {f}, level {l}, {w}x{w})"));
            }
        }
        let s = render_quad(true);
        assert_black_texel("cube size case", &s, true);
    });
}

#[test]
fn texture_completeness_cube_npot_size_level_0() {
    size_cube_black(63, 0, None);
}

#[test]
fn texture_completeness_cube_npot_size_level_1() {
    size_cube_black(31, 1, None);
}

#[test]
fn texture_completeness_cube_npot_size_level_0_pos_x() {
    size_cube_black(63, 0, Some(0));
}

#[test]
fn texture_completeness_cube_npot_size_level_1_neg_x() {
    size_cube_black(31, 1, Some(1));
}

/* ---- a defect the CTS sizes do not reach ------------------------------------------------ */

/// TEXDESC_LOG_SIZE_EXT holds log2 of the base level's size in 8.8 fixed
/// point. Mesa writes the exact value (`etna_log2_fixp88`, Mesa 26.2.1
/// src/gallium/drivers/etnaviv/etnaviv_util.h:57, used at
/// etnaviv_texture_desc.c:200-201 for the GC7000 descriptor). The back end
/// writes the integer part and then a straight line between 2^k and 2^(k+1)
/// (src/vivante/texture.rs `log2_fixp88`, which calls it "close enough for
/// the LOD clamp"). The two agree for a power of two and are within one unit
/// at 127 and 255; for 90 the line gives 6.406 (0x668) where log2(90) = 6.492
/// (0x67d), 0.086 octave low (the most a line can be off). It is NPOT-only,
/// so it is the one NPOT-specific register value that is not Mesa's. What the
/// texture unit does with the field (the LOD calculation, or only a clamp) is
/// not known; the i.MX8MP tree passes the REPEAT cases, whose sizes are all
/// within one unit.
#[test]
#[ignore = "BUG: TEXDESC_LOG_SIZE_EXT is a linear approximation of log2, 0.086 octave low at a size like 90; Mesa writes the exact log2 (src/vivante/texture.rs log2_fixp88)"]
fn bug_log_size_ext_is_a_linear_approximation_of_log2() {
    on_gc7000(|| {
        let (w, h) = (90u32, 70u32);
        let mut t: GLuint = 0;
        // SAFETY: one name; plain GL calls.
        unsafe {
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
        }
        tex_param(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_REPEAT);
        tex_param(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT);
        tex_param(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
        tex_param(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
        tex_image(GL_TEXTURE_2D, 0, w as i32, h as i32, &image(w, h, 0, 0));
        no_error("set texturing state");
        let s = render_quad(false);
        assert_eq!(
            s.desc[D_LOG_SIZE_EXT],
            mesa_log2_fixp88(w) | (mesa_log2_fixp88(h) << 16),
            "TEXDESC_LOG_SIZE_EXT of {w}x{h}: Mesa writes log2 exactly"
        );
    });
}
