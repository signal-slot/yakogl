// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2-es2's `[texture] texture-filter=mipmap` texture on the GC7000UL
//! (Vivante) back end, through the GL API on the host: the descriptor and
//! sampler state its one textured draw is given, and the chain
//! glGenerateMipmap draws.
//!
//! The scene's texture, as glmark2 (pinned `22c527cb`) makes it:
//! `scene-texture.cpp:132-134` takes min GL_LINEAR_MIPMAP_LINEAR and mag
//! GL_LINEAR for `mipmap`, and `texture.cpp` `setup_texture` binds a new 2D
//! texture, sets those two filters and CLAMP_TO_EDGE on S and T, uploads
//! `textures/crate-base.png` (512x512, 8-bit RGB, so GL_RGB /
//! GL_UNSIGNED_BYTE) as level 0, and calls glGenerateMipmap once (its
//! `needs_mipmap`). The draw samples it through glmark2's own
//! `light-basic.vert` / `light-basic-tex.frag`, with the two constants
//! `scene-texture.cpp` adds. The PNG's texels are not needed: a pattern of
//! the same size and format stands in for them.
//!
//! What it pins (what the code does at the commit it was written on):
//! - the chain is the GPU's (`src/vivante/mipmap.rs` `gc_generate_mipmap`:
//!   one chain made, none refused), drawn as nine level draws, each into
//!   level l (the PE colour address) from level l-1 (slot 0's descriptor,
//!   one level, bilinear, CLAMP_TO_EDGE);
//! - the scene draw's descriptor: TYPE 2D, SIZE 512x512, LINEAR_STRIDE 2048,
//!   BASELOD base 0 / max 9 (ten levels), the ten level addresses, each
//!   non-zero, 64-byte aligned, distinct, increasing, inside the texture's
//!   allocation and equal to the level draws' targets; level 0's address
//!   holds the uploaded texels;
//! - the scene draw's sampler: SAMP_CTRL0 MIN LINEAR, MIP LINEAR, MAG
//!   LINEAR, CLAMP_TO_EDGE on U, V (and W = U), and SAMP_LOD_MINMAX min 0,
//!   max 0xfff (GL's default MAX_LOD 1000, saturated).
//!
//! What it cannot see: the GPU. No GPU runs, so the levels the chain's draws
//! write stay as the stand-in allocated them (zero), and a sampled value at
//! a minified LOD cannot be checked on the host: the chain's draws are
//! checked instead (target, source, size and pitch of each level). Pixels
//! are a board question (the i.MX8MP / X2 frame dumps of the scene).
//!
//! HOST-ONLY: API verdicts on the host back end, never pictures and not a
//! board result (tools/cts/README.md 2b).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test vivante_glmark2_mipmap
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test vivante_glmark2_mipmap
//! ```

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

/* ---- the stand-in OSAL: host memory, and the streams the back end submits --------- */

struct Alloc {
    mem: Box<[u64]>,
    size: usize,
}

/// One draw of a submitted stream: every register the stream had written
/// when the draw command came (the stream's own state, as the GPU would hold
/// it), and the descriptors that state names, copied at submit time.
#[derive(Clone)]
struct Draw {
    state: BTreeMap<u32, u32>,
    descs: BTreeMap<u32, Vec<u32>>,
}

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Alloc>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    /// The draws of each kick, in order.
    kicks: Vec<Vec<Draw>>,
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

impl Platform {
    /// `n` bytes at GPU address `gpu`, if one live allocation holds them all.
    fn bytes(&self, gpu: u64, n: usize) -> Option<Vec<u8>> {
        let (base, a) = self.allocs.range(..=gpu).next_back()?;
        let off = (gpu - base) as usize;
        if off + n > a.size {
            return None;
        }
        // SAFETY: inside the allocation (checked just above).
        Some(unsafe { core::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(off), n) }.to_vec())
    }

    /// The allocation holding GPU address `gpu`: (base, size).
    fn alloc_of(&self, gpu: u64) -> Option<(u64, usize)> {
        let (base, a) = self.allocs.range(..=gpu).next_back()?;
        ((gpu - base) < a.size as u64).then_some((*base, a.size))
    }
}

fn dwords(b: &[u8]) -> Vec<u32> {
    b.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect()
}

/// The draws of one stream ([`Draw`]): LOAD_STATE writes accumulate, each
/// DRAW_INSTANCED takes a copy, and the slot descriptors that copy names are
/// read now, before the frame arena that holds them is reused.
fn draws_of(p: &Platform, words: &[u32]) -> Vec<Draw> {
    let mut state = BTreeMap::new();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        match w & 0xf800_0000 {
            GC_OP_LOAD_STATE => {
                let count = ((w >> 16) & 0x3ff) as usize;
                let base = (w & 0xffff) * 4;
                for k in 0..count {
                    if let Some(v) = words.get(i + 1 + k) {
                        state.insert(base + 4 * k as u32, *v);
                    }
                }
                i += 1 + count;
            }
            GC_OP_DRAW_INSTANCED => {
                let mut descs = BTreeMap::new();
                for slot in 0..2 {
                    if let Some(&a) = state.get(&nte_descriptor_addr(slot)) {
                        if let Some(b) = p.bytes(u64::from(a), GC_TEXDESC_BYTES) {
                            descs.insert(a, dwords(&b));
                        }
                    }
                }
                out.push(Draw { state: state.clone(), descs });
                i += 4;
            }
            GC_OP_STALL => i += 2,
            _ => i += 1,
        }
    }
    out
}

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
        let Some(b) = p.bytes(k.stream_gpu, k.stream_bytes as usize) else {
            p.faults.push(format!("the kick's stream 0x{:x} is no allocation", k.stream_gpu));
            return -22;
        };
        let d = draws_of(p, &dwords(&b));
        p.kicks.push(d);
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

/* ---- a fresh context, back end and OSAL per case (800x600, glmark2's size) ------------ */

fn on_gc7000(case: impl FnOnce()) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(800, 600);
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
        p.kicks.clear();
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

/// glmark2's `data/shaders/light-basic.vert`, with the two constants
/// `scene-texture.cpp` adds (`lightPosition` 20, 20, 10, 1 and
/// `materialDiffuse` 0.7, 0.7, 0.7, 1), as `ShaderSource::add_const` writes
/// them.
const VS: &str = concat!(
    "const vec4 LightSourcePosition = vec4(20.0, 20.0, 10.0, 1.0);\n",
    "const vec4 MaterialDiffuse = vec4(0.7, 0.7, 0.7, 1.0);\n",
    "attribute vec3 position;\nattribute vec3 normal;\nattribute vec2 texcoord;\n",
    "uniform mat4 ModelViewProjectionMatrix;\nuniform mat4 NormalMatrix;\n",
    "varying vec4 Color;\nvarying vec2 TextureCoord;\n",
    "void main(void)\n{\n",
    "    vec3 N = normalize(vec3(NormalMatrix * vec4(normal, 1.0)));\n",
    "    vec3 L = normalize(LightSourcePosition.xyz);\n",
    "    float diffuse = max(dot(N, L), 0.0);\n",
    "    Color = vec4(diffuse * MaterialDiffuse.rgb, MaterialDiffuse.a);\n",
    "    TextureCoord = texcoord;\n",
    "    gl_Position = ModelViewProjectionMatrix * vec4(position, 1.0);\n",
    "}\n"
);
/// glmark2's `data/shaders/light-basic-tex.frag`, behind the default
/// precision glmark2's ShaderSource puts first in an ES build.
const FS: &str = concat!(
    "precision mediump float;\n",
    "uniform sampler2D MaterialTexture0;\n",
    "varying vec4 Color;\nvarying vec2 TextureCoord;\n",
    "void main(void)\n{\n",
    "    vec4 texel = texture2D(MaterialTexture0, TextureCoord);\n",
    "    gl_FragColor = texel * Color;\n",
    "}\n"
);

/// The scene's texture: crate-base's size and format.
const SIZE: u32 = 512;
/// floor(log2(512)) + 1
const LEVELS: u32 = 10;
/// The GL_RGB texel the stand-in image has at (x, y).
fn texel(x: u32, y: u32) -> [u8; 3] {
    [(x & 0xff) as u8, (y & 0xff) as u8, ((x ^ y) >> 1) as u8]
}

/// glmark2 `texture.cpp` `setup_texture` for `min`/`mag` on unit 0, and
/// glGenerateMipmap when `min` is a mipmap filter (its `needs_mipmap`).
fn setup_texture(min: GLenum, mag: GLenum) -> GLuint {
    let mut pixels = Vec::with_capacity((SIZE * SIZE * 3) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            pixels.extend_from_slice(&texel(x, y));
        }
    }
    let mut t: GLuint = 0;
    // SAFETY: plain GL calls; `pixels` holds the 512x512 RGB image.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, min as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, mag as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGB as GLint,
            SIZE as GLsizei,
            SIZE as GLsizei,
            0,
            GL_RGB,
            GL_UNSIGNED_BYTE,
            pixels.as_ptr().cast(),
        );
        no_error("glTexImage2D");
        if min != GL_NEAREST && min != GL_LINEAR {
            glGenerateMipmap(GL_TEXTURE_2D);
            no_error("glGenerateMipmap");
        }
    }
    t
}

/// position (3), normal (3), texcoord (2): a quad facing the light, a
/// quarter of the 800x600 canvas, so the 512x512 texture is minified.
static QUAD: [f32; 32] = [
    -0.25, -0.25, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, //
    0.25, -0.25, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, //
    -0.25, 0.25, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, //
    0.25, 0.25, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0,
];
const IDENTITY: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

/// The scene's one textured draw, with glmark2's program, finished. Its
/// draw as the kept stream holds it.
fn scene_draw() -> Draw {
    let kicks0 = with_platform(|p| p.kicks.len());
    // SAFETY: plain GL calls; QUAD and IDENTITY outlive the draw.
    unsafe {
        glViewport(0, 0, 800, 600);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
        glBindAttribLocation(p, 0, c"position".as_ptr());
        glBindAttribLocation(p, 1, c"normal".as_ptr());
        glBindAttribLocation(p, 2, c"texcoord".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "glmark2's light-basic program did not link");
        glUseProgram(p);
        glUniformMatrix4fv(glGetUniformLocation(p, c"ModelViewProjectionMatrix".as_ptr()), 1, 0, IDENTITY.as_ptr());
        glUniformMatrix4fv(glGetUniformLocation(p, c"NormalMatrix".as_ptr()), 1, 0, IDENTITY.as_ptr());
        glUniform1i(glGetUniformLocation(p, c"MaterialTexture0".as_ptr()), 0);
        let base = QUAD.as_ptr();
        for (loc, n, off) in [(0u32, 3, 0usize), (1, 3, 3), (2, 2, 6)] {
            glEnableVertexAttribArray(loc);
            glVertexAttribPointer(loc, n, GL_FLOAT, 0, 32, base.add(off) as *const c_void);
        }
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    }
    no_error("draw");
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    no_error("glFinish");
    with_platform(|p| {
        let d: Vec<&Draw> = p.kicks[kicks0..].iter().flatten().collect();
        assert_eq!(d.len(), 1, "the scene's frame holds one draw");
        d[0].clone()
    })
}

/* ---- reading a draw --------------------------------------------------------------------- */

impl Draw {
    fn reg(&self, reg: u32, name: &str) -> u32 {
        *self.state.get(&reg).unwrap_or_else(|| panic!("{name} (0x{reg:x}) was never written"))
    }
    /// Slot `slot`'s descriptor, as it was when the stream was submitted.
    fn desc(&self, slot: u32) -> &[u32] {
        let a = self.reg(nte_descriptor_addr(slot), "NTE_DESCRIPTOR_ADDR");
        assert_ne!(a, 0, "no descriptor for slot {slot}");
        self.descs.get(&a).unwrap_or_else(|| panic!("slot {slot}'s descriptor 0x{a:x} is no allocation"))
    }
    fn samp_ctrl0(&self, slot: u32) -> u32 {
        self.reg(nte_descriptor_samp_ctrl0(slot), "NTE_DESCRIPTOR_SAMP_CTRL0")
    }
}

fn word(desc: &[u32], off: u32) -> u32 {
    desc[off as usize / 4]
}

/// The level addresses (descriptor words 0..15) of the first `n` levels.
fn lod_addrs(desc: &[u32], n: u32) -> Vec<u32> {
    (0..n).map(|l| word(desc, texdesc_lod_addr(l))).collect()
}

/// SAMP_CTRL0's fields: (UWRAP, VWRAP, WWRAP, MIN, MIP, MAG).
fn ctrl0_fields(c: u32) -> [u32; 6] {
    [c & 7, (c >> 3) & 7, (c >> 6) & 7, (c >> 9) & 3, (c >> 11) & 3, (c >> 13) & 3]
}

/// The GL extent of level `l` of the 512x512 chain.
fn level_size(l: u32) -> u32 {
    (SIZE >> l).max(1)
}

/// Bytes level `l` takes in the chain: every level at level 0's pitch
/// (`src/vivante/texture.rs` `build_descriptor`, `mipmap.rs` `chain_layout`).
const PITCH: u32 = SIZE * 4;

fn chains() -> (u32, u32) {
    let (mut made, mut refused) = (0u32, 0u32);
    // SAFETY: two writable counters.
    unsafe { yakogl::vivante::mipmap::gc_backend_mip_chains(&mut made, &mut refused) };
    (made, refused)
}

/* ---- the tests ------------------------------------------------------------------------- */

/// The scene's draw: what the sampler and the descriptor say about the chain.
#[test]
fn the_scene_draw_samples_a_ten_level_trilinear_chain() {
    on_gc7000(|| {
        setup_texture(GL_LINEAR_MIPMAP_LINEAR, GL_LINEAR);
        let d = scene_draw();
        let desc = d.desc(0);

        /* the sampler: min LINEAR, mip LINEAR, mag LINEAR, clamped */
        let c = d.samp_ctrl0(0);
        assert_eq!(
            ctrl0_fields(c),
            [
                GC_TEX_WRAP_CLAMP_TO_EDGE,
                GC_TEX_WRAP_CLAMP_TO_EDGE,
                GC_TEX_WRAP_CLAMP_TO_EDGE,
                GC_TEX_FILTER_LINEAR,
                GC_TEX_FILTER_LINEAR,
                GC_TEX_FILTER_LINEAR
            ],
            "SAMP_CTRL0 0x{c:08x}: U, V, W wrap, MIN, MIP, MAG"
        );
        assert_ne!(c & GC_SAMP_CTRL0_UNK21, 0, "SAMP_CTRL0 UNK21");
        assert_ne!(c & GC_SAMP_INT_FILTER, 0, "an RGBA8 2D texture takes INT_FILTER (Mesa's rule)");
        assert_eq!(d.reg(nte_descriptor_samp_ctrl1(0), "SAMP_CTRL1"), GC_SAMP_CTRL1_UNK1, "SAMP_CTRL1");
        /* the sampler's LOD range: GL's MIN_LOD -1000 / MAX_LOD 1000 defaults,
         * written as 0 and the field's saturated 0xfff */
        let lod = d.reg(nte_descriptor_samp_lod_minmax(0), "SAMP_LOD_MINMAX");
        assert_eq!(lod, samp_lod_max(GC_SAMP_LOD_FP58_MAX) | samp_lod_min(0), "SAMP_LOD_MINMAX 0x{lod:08x}");
        assert_eq!(d.reg(nte_descriptor_samp_lod_bias(0), "SAMP_LOD_BIAS"), 0, "SAMP_LOD_BIAS");

        /* the descriptor: a linear 512x512 2D chain of ten levels */
        let config0 = word(desc, GC_TEXDESC_CONFIG0);
        assert_eq!(config0 & 7, GC_TEX_TYPE_2D, "CONFIG0 0x{config0:08x} TYPE");
        assert_eq!((config0 >> 20) & 3, GC_TEX_ADDRESSING_LINEAR, "CONFIG0 0x{config0:08x} ADDRESSING");
        assert_eq!(word(desc, GC_TEXDESC_SIZE), texdesc_size_width(SIZE) | texdesc_size_height(SIZE), "SIZE");
        assert_eq!(word(desc, GC_TEXDESC_LOG_SIZE_EXT), (9 << 8) | (9 << 24), "LOG_SIZE_EXT: log2 512 = 9.0 in 8.8");
        assert_eq!(word(desc, GC_TEXDESC_LINEAR_STRIDE), PITCH, "LINEAR_STRIDE: 512 RGBA8 texels");
        let baselod = word(desc, GC_TEXDESC_BASELOD);
        assert_eq!(
            baselod,
            texdesc_baselod_base(0) | texdesc_baselod_max(LEVELS - 1),
            "BASELOD 0x{baselod:08x}: base 0, max {} (a chain of {LEVELS})",
            LEVELS - 1
        );

        /* the ten level addresses, and none past them */
        let a = lod_addrs(desc, 16);
        for (l, &x) in a.iter().enumerate().take(LEVELS as usize) {
            assert_ne!(x, 0, "level {l}'s address is 0");
            assert_eq!(x as usize % GC_TEXDESC_LOD_ALIGN, 0, "level {l}'s address 0x{x:x} is not 64-byte aligned");
        }
        assert!(a[LEVELS as usize..].iter().all(|&x| x == 0), "addresses past the last level: {:x?}", a);
        let (base, size) = with_platform(|p| p.alloc_of(u64::from(a[0]))).expect("level 0's address is no allocation");
        for l in 0..LEVELS as usize {
            let at = u64::from(a[l]);
            let bytes = u64::from(PITCH * level_size(l as u32));
            assert!(
                at >= base && at + bytes <= base + size as u64,
                "level {l} at 0x{at:x} (+{bytes}) is not inside the texture's allocation 0x{base:x} (+{size})"
            );
            if l > 0 {
                /* each level right after the one before, at the chain's
                 * pitch, on the next 64-byte boundary */
                let prev = u64::from(a[l - 1]) + u64::from(PITCH * level_size(l as u32 - 1));
                assert_eq!(at, prev.next_multiple_of(64), "level {l}'s address");
            }
        }
        let mut sorted = a[..LEVELS as usize].to_vec();
        sorted.dedup();
        assert_eq!(sorted.len(), LEVELS as usize, "level addresses are distinct: {:x?}", &a[..LEVELS as usize]);

        /* level 0's address holds the upload, as R,G,B,1 texels */
        let row0 = with_platform(|p| p.bytes(u64::from(a[0]), (PITCH * 2) as usize)).unwrap();
        for (x, y) in [(0u32, 0u32), (1, 0), (37, 0), (511, 0), (5, 1), (300, 1)] {
            let o = (y * PITCH + x * 4) as usize;
            let [r, g, b] = texel(x, y);
            assert_eq!(&row0[o..o + 4], &[r, g, b, 0xff], "level 0 texel ({x}, {y})");
        }
    });
}

/// glGenerateMipmap: the GPU draws levels 1 to 9, each from the level above,
/// into the addresses the scene's descriptor reads them from.
#[test]
fn glgeneratemipmap_draws_each_level_from_the_one_above() {
    on_gc7000(|| {
        let (made0, refused0) = chains();
        let kicks0 = with_platform(|p| p.kicks.len());
        setup_texture(GL_LINEAR_MIPMAP_LINEAR, GL_LINEAR);
        let (made, refused) = chains();
        assert_eq!((made - made0, refused - refused0), (1, 0), "chains made and refused by glGenerateMipmap");
        let chain: Vec<Draw> = with_platform(|p| p.kicks[kicks0..].iter().flatten().cloned().collect());
        assert_eq!(chain.len(), LEVELS as usize - 1, "one draw a level above 0");

        let d = scene_draw();
        let a = lod_addrs(d.desc(0), LEVELS);
        for (i, ld) in chain.iter().enumerate() {
            let l = i as u32 + 1;
            /* the target: level l, at the chain's pitch */
            assert_eq!(ld.reg(pe_pipe_color_addr(0), "PE_PIPE_COLOR_ADDR(0)"), a[l as usize], "level {l}'s target");
            assert_eq!(ld.reg(GC_PE_COLOR_STRIDE, "PE_COLOR_STRIDE"), PITCH, "level {l}'s target pitch");
            /* the source: level l - 1, one level, bilinear, clamped */
            let src = ld.desc(0);
            let s = level_size(l - 1);
            assert_eq!(word(src, texdesc_lod_addr(0)), a[l as usize - 1], "level {l}'s source address");
            assert_eq!(word(src, texdesc_lod_addr(1)), 0, "level {l}'s source is one level");
            assert_eq!(
                word(src, GC_TEXDESC_SIZE),
                texdesc_size_width(s) | texdesc_size_height(s),
                "level {l}'s source size"
            );
            assert_eq!(word(src, GC_TEXDESC_LINEAR_STRIDE), PITCH, "level {l}'s source pitch");
            assert_eq!(word(src, GC_TEXDESC_BASELOD), 0, "level {l}'s source BASELOD");
            let f = ctrl0_fields(ld.samp_ctrl0(0));
            assert_eq!(
                f,
                [
                    GC_TEX_WRAP_CLAMP_TO_EDGE,
                    GC_TEX_WRAP_CLAMP_TO_EDGE,
                    GC_TEX_WRAP_CLAMP_TO_EDGE,
                    GC_TEX_FILTER_LINEAR,
                    GC_TEX_FILTER_NONE,
                    GC_TEX_FILTER_LINEAR
                ],
                "level {l}'s source sampler"
            );
        }
    });
}

/// The scene's `nearest` and `linear` twins (the same texture with no
/// glGenerateMipmap) sample one level with no mip filter: what tells the
/// three `[texture]` scenes' sampler state apart.
#[test]
fn nearest_and_linear_sample_one_level() {
    for (min, mag, want) in [
        (GL_NEAREST, GL_NEAREST, [GC_TEX_FILTER_NEAREST, GC_TEX_FILTER_NONE, GC_TEX_FILTER_NEAREST]),
        (GL_LINEAR, GL_LINEAR, [GC_TEX_FILTER_LINEAR, GC_TEX_FILTER_NONE, GC_TEX_FILTER_LINEAR]),
    ] {
        on_gc7000(|| {
            setup_texture(min, mag);
            let d = scene_draw();
            let f = ctrl0_fields(d.samp_ctrl0(0));
            assert_eq!([f[3], f[4], f[5]], want, "min 0x{min:x}: MIN, MIP, MAG");
            let desc = d.desc(0);
            assert_eq!(word(desc, GC_TEXDESC_BASELOD), 0, "min 0x{min:x}: BASELOD (one level)");
            assert_ne!(word(desc, texdesc_lod_addr(0)), 0, "min 0x{min:x}: level 0's address");
            assert_eq!(word(desc, texdesc_lod_addr(1)), 0, "min 0x{min:x}: no level 1");
        });
    }
}
