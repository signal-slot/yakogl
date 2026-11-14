// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): a vertex-stage RGBA32 binding (item gc-w128-vertex;
//! docs/notes/gc-128bit-texture-sampling.md, "What is left").
//!
//! A 128-bit texture is sampled as two G32R32 planes and a companion slot.
//! The vertex stage takes the same path as the fragment stage, with three
//! differences this file pins: its slots start at `VS_SAMPLER_BASE` (after
//! the fragment stage's slots and companions), its key is numbered by the
//! stage's own `TEX_ID` (the slot minus that base), and it is sampled from a
//! tiled copy (`tiled_copy::wanted_vs`: every 2D texture the vertex stage
//! reads, Mesa golden mode 26), which for a 16-byte class is both planes.
//!
//! - The tests that always run drive the draw's key as the GL core builds it
//!   (`wide128_companions`, `wide128_planes`) and the compile of the vertex
//!   stage under it (`wide128_variant`): one companion for an RGBA32F and
//!   for an RGBA32UI binding, two look-ups in the compiled stage, the
//!   vertex and the fragment stage keyed apart for one texture bound to
//!   both, and the slot arithmetic of that draw.
//! - The draw tests go through the GL API on the stand-in OSAL and read the
//!   descriptors and both planes out of the submitted stream. They need the
//!   128-bit classes in the sampled mask, whose test hook
//!   (`backend::TEST_EXTRA_SAMPLED`) is `#[cfg(test)]` of the library and so
//!   out of reach of an integration test: until GC_SAMPLED_TEXFMTS holds the
//!   classes (item wide128-flip) they check the closed state and say so on
//!   stderr; from the flip on they run for real. Before the flip they were
//!   run with the three classes added to GC_SAMPLED_TEXFMTS locally.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_vertex_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::online::backend::{forget_compiled, SHC_ONLINE};
use yakogl::shaderc::online::vivante::decode::decode_instr;
use yakogl::shaderc::{
    release_shader, wide128_planes, wide128_sensitivity, wide128_variant, LodKey, ShcShader, Wide128Planes,
    SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX,
};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;
use yakogl::vivante::texture::{is_wide128, stage_slots_fit, wide128_sampled};

const COMMON_3D: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/driver/src/vivante/hw/common_3d.xml.h"));

/// The value of `#define <name>` in the vendored common_3d.xml.h.
fn rnndb(name: &str) -> u32 {
    let line = COMMON_3D
        .lines()
        .find(|l| l.starts_with("#define") && l.split_whitespace().nth(1) == Some(name))
        .unwrap_or_else(|| panic!("{name} is in common_3d.xml.h"));
    let v = line.split_whitespace().nth(2).expect("a value");
    u32::from_str_radix(v.trim_start_matches("0x"), 16).expect("hex")
}

/// Name, class, GL internal format, sampler type, vertex-output type, the
/// Mesa format word of both planes.
const WIDE: [(&str, u8, u32, &str, &str, &str); 3] = [
    ("RGBA32F", GLES_TEXFMT_RGBA32F, GL_RGBA32F, "sampler2D", "vec4", "TEXTURE_FORMAT_EXT_G32R32F"),
    ("RGBA32UI", GLES_TEXFMT_RGBA32UI, GL_RGBA32UI, "usampler2D", "uvec4", "TEXTURE_FORMAT_EXT_G32R32I"),
    ("RGBA32I", GLES_TEXFMT_RGBA32I, GL_RGBA32I, "isampler2D", "ivec4", "TEXTURE_FORMAT_EXT_G32R32I"),
];

/// Is the class in the back end's sampled mask (so the GL core stores it)?
fn open(format: u8) -> bool {
    (sampled_texfmts() as u64) & (gles_texfmt_bit(format) as u64) != 0
}

fn align4(v: u32) -> u32 {
    (v + 3) & !3
}

/* ---- the key and the compile, as the GL core makes them ----------------- */

/// Held across every compile and every read of its code: the plane key is
/// process-wide, and `forget_compiled` resets the arena the code lives in.
static COMPILING: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    let g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    forget_compiled();
    g
}

fn text(log: &[c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

fn compile(stage: u32, src: &str) -> ShcShader {
    let src = CString::new(src).unwrap();
    let mut sh = ShcShader::ZERO;
    let mut log = [0 as c_char; 512];
    let compile = SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, src.as_ptr(), &mut sh, log.as_mut_ptr(), log.len()) };
    assert_eq!(rc, 0, "{}", text(&log));
    sh
}

fn variant(sh: &ShcShader, planes: Wide128Planes) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 512];
    match wide128_variant(sh, planes, LodKey::NONE, &mut out, &mut log) {
        0 => Ok(out),
        _ => Err(text(&log)),
    }
}

/// The `TEX_ID` of every look-up of `sh`, in program order.
fn units(sh: &ShcShader) -> Vec<(&'static str, u32)> {
    sh.usc_words()
        .chunks(4)
        .map(|c| decode_instr(&c.try_into().unwrap()).expect("decodes"))
        .filter(|i| i.op().name.starts_with("tex") || i.op().name == "txf")
        .map(|i| (i.op().name, i.get("TEX_ID").expect("a look-up names its unit")))
        .collect()
}

/// A vertex shader reading `s` once, by an explicit level, into a flat output.
fn vs_src(sampler: &str, out: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform highp {sampler} s;\nin vec4 p;\n\
         flat out highp {out} v;\nvoid main() {{ v = textureLod(s, p.xy, 0.0); gl_Position = p; }}\n"
    )
}

/// What the draw asks of the texture: two planes, whatever the mask.
fn wide(t: &GlesTextureBind) -> bool {
    is_wide128(t.format)
}

/// A texture of `format` bound to `unit`.
fn bind(unit: u8, format: u8) -> GlesTextureBind {
    GlesTextureBind { unit, format, width: 8, height: 8, mip_levels: 1, layers: 1, ..GlesTextureBind::ZERO }
}

/// A vertex-stage RGBA32F and a vertex-stage RGBA32UI binding each get a
/// vertex-stage key with one companion, and the fragment stage none; the
/// vertex stage compiled under the key has two look-ups, the second on the
/// companion slot in the stage's own numbering.
#[test]
fn a_vertex_binding_gets_a_vertex_key_with_one_companion() {
    let _g = serial();
    let none = ShcShader::ZERO;
    for (name, class, _, sampler, out, _) in WIDE {
        let vs = compile(SHC_STAGE_VERTEX, &vs_src(sampler, out));
        let sense = wide128_sensitivity(&vs);
        assert_eq!(sense, 1, "{name}: the vertex stage reads binding 0");
        let plain = units(&vs);
        assert_eq!(plain.len(), 1, "{name}: {plain:?}");

        /* nothing bound in the fragment stage: the vertex stage's base is 0 */
        let c = wide128_companions(&[bind(0, class)], &vs, &[], &none, wide);
        assert_eq!(c[0][0], Some(1), "{name}: one slot for the unit, the companion right after");
        assert!(c[0][1..].iter().all(|s| s.is_none()), "{name}: only the bound unit");
        assert!(c[1].iter().all(|s| s.is_none()), "{name}: the fragment stage has no key");

        let key = wide128_planes(sense, &c[0]);
        assert_eq!(key, Wide128Planes::NONE.with(0, 1), "{name}");
        assert!(wide128_planes(0, &c[0]).is_empty(), "{name}: a stage that reads nothing is not keyed");

        let v = variant(&vs, key).unwrap_or_else(|e| panic!("{name}: the vertex stage splits: {e}"));
        let split = units(&v);
        assert_eq!(split.len(), 2, "{name}: two Tex, {split:?}");
        assert_eq!(split[0], plain[0], "{name}: the R,G plane on the binding's own unit");
        assert_eq!(split[1], (plain[0].0, plain[0].1 + 1), "{name}: the B,A plane on the companion");
        assert_ne!(v.priv_, vs.priv_, "{name}: a record of its own");
        release_shader(&v);
        release_shader(&vs);
    }
}

/// A binding that is not two planes (RGBA8, RGBA16F, R32F) has no companion
/// in the vertex stage either, and the plain vertex stage keeps its one Tex.
#[test]
fn a_one_plane_vertex_binding_is_not_keyed() {
    let _g = serial();
    let none = ShcShader::ZERO;
    let vs = compile(SHC_STAGE_VERTEX, &vs_src("sampler2D", "vec4"));
    for class in [GLES_TEXFMT_RGBA8, GLES_TEXFMT_RGBA16F, GLES_TEXFMT_R32F] {
        let c = wide128_companions(&[bind(0, class)], &vs, &[], &none, wide);
        assert!(c.iter().flatten().all(|s| s.is_none()), "class {class}");
        assert!(wide128_planes(wide128_sensitivity(&vs), &c[0]).is_empty(), "class {class}");
    }
    assert_eq!(units(&vs).len(), 1);
    release_shader(&vs);
}

/// One texture sampled in both stages: the stages are keyed apart. The
/// fragment stage's slots are 0 (unit) and 1 (companion), the vertex stage's
/// base 2 with its unit at 2 and its companion at 3; each stage's key is
/// numbered by its own TEX_ID, so both read `1`, and each compiled stage has
/// two look-ups.
#[test]
fn one_texture_in_both_stages_is_keyed_in_each() {
    let _g = serial();
    for (name, class, _, sampler, out, _) in WIDE {
        let vs = compile(SHC_STAGE_VERTEX, &vs_src(sampler, out));
        let fs = compile(
            SHC_STAGE_FRAGMENT,
            &format!(
                "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform highp {sampler} s;\nflat in highp {out} v;\n\
                 out highp vec4 o;\nvoid main() {{ o = vec4(v) + vec4(texelFetch(s, ivec2(0), 0)); }}\n"
            ),
        );
        let (vsense, fsense) = (wide128_sensitivity(&vs), wide128_sensitivity(&fs));
        assert_eq!((vsense, fsense), (1, 1), "{name}");
        let t = [bind(0, class)];
        let c = wide128_companions(&t, &vs, &t, &fs, wide);
        assert_eq!((c[0][0], c[1][0]), (Some(1), Some(1)), "{name}: each stage one companion");
        let (vk, fk) = (wide128_planes(vsense, &c[0]), wide128_planes(fsense, &c[1]));
        assert_eq!((vk, fk), (Wide128Planes::NONE.with(0, 1), Wide128Planes::NONE.with(0, 1)), "{name}");

        let (vv, fv) = (variant(&vs, vk).expect("vertex splits"), variant(&fs, fk).expect("fragment splits"));
        assert_eq!(units(&vv).len(), 2, "{name}: vertex stage, two Tex");
        let fu = units(&fv);
        assert_eq!(fu.iter().map(|u| u.1).collect::<Vec<_>>(), [0, 1], "{name}: fragment stage, two Tex: {fu:?}");

        /* the draw's slots: 0,1 fragment; 2,3 vertex; all inside the array */
        assert_eq!(stage_slots_fit(1, 1, 1), Ok(()));
        /* the vertex companion alone moves when only the vertex stage is wide */
        let c = wide128_companions(&t, &vs, &t, &fs, |x| wide(x) && x.unit == 0);
        assert_eq!(c[0][0], Some(1));
        release_shader(&vv);
        release_shader(&fv);
        release_shader(&vs);
        release_shader(&fs);
    }
}

/// With the fragment stage wide on two units, the vertex stage's key keeps
/// its own numbering: the vertex slot base moves by the fragment companions,
/// the stage-relative companion does not.
#[test]
fn the_vertex_key_is_numbered_from_its_own_base() {
    let none = ShcShader::ZERO;
    let fs: Vec<_> = (0..3).map(|u| bind(u, GLES_TEXFMT_RGBA32F)).collect();
    for vs_unit in 0..3u8 {
        let vs = [bind(vs_unit, GLES_TEXFMT_RGBA32UI)];
        let c = wide128_companions(&vs, &none, &fs, &none, wide);
        /* the unit's slot is vs_unit, the stage has vs_unit + 1 of them: the companion follows */
        assert_eq!(c[0][vs_unit as usize], Some(vs_unit + 1), "vertex unit {vs_unit}");
        assert_eq!(&c[1][..3], &[Some(3), Some(4), Some(5)], "fragment companions in unit order");
    }
    /* two wide vertex bindings: ranked in unit order, after the stage's slots */
    let vs = [bind(2, GLES_TEXFMT_RGBA32F), bind(0, GLES_TEXFMT_RGBA32I)];
    let c = wide128_companions(&vs, &none, &[], &none, wide);
    assert_eq!((c[0][0], c[0][2]), (Some(3), Some(4)));
}

/* ---- the stand-in OSAL (as tests/es3_wide128_planes_gc.rs) -------------- */

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

fn words_at(gpu: u64, n: usize) -> Vec<u32> {
    let b = read_gpu(gpu, n * 4);
    let (words, _) = b.as_chunks::<4>();
    words.iter().map(|c| u32::from_le_bytes(*c)).collect()
}

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

/// Run `case` with the Vivante back end up on the stand-in OSAL and a fresh
/// 64x64 context.
fn on_gc7000(case: impl FnOnce()) {
    let _c = serial();
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
        /* leaked, not freed, as in gc_npot_repeat.rs: addresses keep counting up */
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

/* ---- GL plumbing -------------------------------------------------------- */

fn no_error(what: &str) {
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
}

fn shader(ty: GLenum, src: &str) -> GLuint {
    let text = CString::new(src).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok != 1 {
            let mut log = [0 as c_char; 512];
            glGetShaderInfoLog(s, 512, core::ptr::null_mut(), log.as_mut_ptr());
            panic!("{text:?} did not compile: {}", self::text(&log));
        }
        s
    }
}

/// A program whose vertex stage reads `s` and whose fragment stage reads it
/// too when `both`.
fn program(sampler: &str, out: &str, both: bool) -> GLuint {
    let vs = vs_src(sampler, out);
    let fs = if both {
        format!(
            "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform highp {sampler} s;\n\
             flat in highp {out} v;\nout vec4 o;\nvoid main() {{ o = vec4(v) + vec4(texelFetch(s, ivec2(0), 0)); }}\n"
        )
    } else {
        format!("#version 300 es\nprecision highp float;\nflat in highp {out} v;\nout vec4 o;\nvoid main() {{ o = vec4(v); }}\n")
    };
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, &vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, &fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok != 1 {
            let mut log = [0 as c_char; 512];
            glGetProgramInfoLog(p, 512, core::ptr::null_mut(), log.as_mut_ptr());
            panic!("link failed: {}", text(&log));
        }
        p
    }
}

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// The words of slot `slot`'s descriptor in the last kick, if it wrote one.
fn descriptor(stream: &[(u32, u32)], slot: u32) -> Option<Vec<u32>> {
    let a = stream.iter().rev().find(|(r, _)| *r == nte_descriptor_addr(slot))?.1;
    (a != 0).then(|| words_at(u64::from(a), GC_TEXDESC_BYTES / 4))
}

/// Every slot of the 32-entry array the kick wrote a descriptor to.
fn written_slots(stream: &[(u32, u32)]) -> Vec<u32> {
    (0..32).filter(|&s| descriptor(stream, s).is_some()).collect()
}

/// One quad with program `p` (its sampler on unit 0): the last kick's writes.
fn draw(p: GLuint) -> Vec<(u32, u32)> {
    let kicks0 = with_platform(|p| p.kicks.len());
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
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
    writes(&stream)
}

/* ---- the texels and the model ------------------------------------------- */

const W: u32 = 13;
const H: u32 = 6;

/// The four words of texel (x, y): a hash, the finite floats [1, 2) for RGBA32F.
fn texel(float: bool, x: u32, y: u32) -> [u32; 4] {
    let mut out = [0u32; 4];
    for (c, o) in out.iter_mut().enumerate() {
        let mut h =
            x.wrapping_mul(0x85eb_ca6b) ^ y.wrapping_mul(0xc2b2_ae35) ^ (c as u32 + 1).wrapping_mul(0x27d4_eb2f);
        h ^= h >> 15;
        h = h.wrapping_mul(0x2c1b_3c6d);
        h ^= h >> 12;
        *o = if float { 0x3f80_0000 | (h & 0x007f_ffff) } else { h };
    }
    out
}

/// A single-level `W`x`H` texture of the class on unit 0, NEAREST.
fn upload(class: &(&str, u8, u32, &str, &str, &str)) -> Vec<[u32; 4]> {
    let float = class.1 == GLES_TEXFMT_RGBA32F;
    let m: Vec<[u32; 4]> = (0..H).flat_map(|y| (0..W).map(move |x| texel(float, x, y))).collect();
    let bytes: Vec<u8> = m.iter().flatten().flat_map(|w| w.to_le_bytes()).collect();
    let (ty, fmt) = match class.1 {
        GLES_TEXFMT_RGBA32F => (GL_FLOAT, GL_RGBA),
        GLES_TEXFMT_RGBA32I => (GL_INT, GL_RGBA_INTEGER),
        _ => (GL_UNSIGNED_INT, GL_RGBA_INTEGER),
    };
    // SAFETY: plain GL calls; the image holds `W * H` 16-byte texels.
    unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexImage2D(GL_TEXTURE_2D, 0, class.2 as GLint, W as GLsizei, H as GLsizei, 0, fmt, ty, bytes.as_ptr().cast());
    }
    no_error("glTexImage2D");
    m
}

/// The pair of descriptors at slots (`a`, `b`) against Mesa's for a 128-bit
/// class, and both planes against the model: the copy keeps 16 bytes a texel
/// (8 in each plane, the second half-way into the level), byte-exact for
/// every texel of the image. Returns the level-0 addresses of the two planes.
fn check_pair(
    what: &str,
    class: &(&str, u8, u32, &str, &str, &str),
    stream: &[(u32, u32)],
    a: u32,
    b: u32,
    m: &[[u32; 4]],
) -> [u32; 2] {
    let d0 = descriptor(stream, a).unwrap_or_else(|| panic!("{what}: no descriptor in slot {a}"));
    let d1 = descriptor(stream, b).unwrap_or_else(|| panic!("{what}: no companion descriptor in slot {b}"));
    let ext = rnndb(class.5);
    for (p, d) in [&d0, &d1].into_iter().enumerate() {
        let c0 = d[GC_TEXDESC_CONFIG0 as usize / 4];
        let c1 = d[GC_TEXDESC_CONFIG1 as usize / 4];
        assert_eq!((c0 >> 13) & 0x1f, 0, "{what}: plane {p}: no CONFIG0 format");
        assert_eq!((c0 >> 20) & 3, GC_TEX_ADDRESSING_TILED, "{what}: plane {p}: TILED");
        assert_eq!(c1 & 0x3f, ext, "{what}: plane {p}: {}", class.5);
        assert_eq!(d[GC_TEXDESC_SIZE as usize / 4], texdesc_size_width(W) | texdesc_size_height(H), "{what}: size");
        assert_eq!(
            d[GC_TEXDESC_LINEAR_STRIDE as usize / 4],
            align4(W) * 8,
            "{what}: plane {p}: a row of G32R32 texels"
        );
    }
    for i in 16..d0.len() {
        assert_eq!(d0[i], d1[i], "{what}: descriptor word {i}: the companion is a copy");
    }
    let (a0, a1) = (d0[texdesc_lod_addr(0) as usize / 4], d1[texdesc_lod_addr(0) as usize / 4]);
    assert_eq!(u64::from(a0) % 4096, 0, "{what}: the copy starts on a page");
    let plane = align4(W) * align4(H) * 8;
    assert_eq!(a1, a0 + plane, "{what}: 16 bytes a texel: plane 1 half-way into the level");
    let (p0, p1) = (words_at(u64::from(a0), plane as usize / 4), words_at(u64::from(a1), plane as usize / 4));
    for y in 0..H {
        for x in 0..W {
            let tile = (y / 4) * (align4(W) / 4) + x / 4;
            let at = (tile * 16 + (y % 4) * 4 + x % 4) as usize * 2;
            let want = m[(y * W + x) as usize];
            assert_eq!([p0[at], p0[at + 1]], want[..2], "{what}: R,G of ({x}, {y}) in plane 0");
            assert_eq!([p1[at], p1[at + 1]], want[2..], "{what}: B,A of ({x}, {y}) in plane 1");
        }
    }
    [a0, a1]
}

/// Did the GL core store the class as planes (RGBA32F is held as halves by
/// `sized_format` whatever the mask says)? Says so when it did not.
fn stored_as_planes(class: &(&str, u8, u32, &str, &str, &str), stream: &[(u32, u32)], slot: u32) -> bool {
    let d = descriptor(stream, slot).unwrap_or_else(|| panic!("{}: no descriptor in slot {slot}", class.0));
    let planes = d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f == rnndb(class.5);
    if !planes {
        eprintln!("{}: the GL core does not store the class as planes (texture.rs sized_format); not checked", class.0);
    }
    planes
}

/// A draw whose vertex stage alone samples the texture: the key is the
/// vertex stage's, both plane descriptors are in the vertex sampler slots
/// (the unit's slot and the one after it), and the tiled copy keeps both
/// planes at 8 bytes a texel. A class the mask does not hold is not sampled
/// as planes (and not stored: the GL core has no 128-bit class for it).
#[test]
fn a_vertex_stage_draw_emits_both_plane_descriptors() {
    for class in &WIDE {
        if !open(class.1) {
            eprintln!("{}: not in GC_SAMPLED_TEXFMTS; the vertex draw is not exercised (wide128-flip)", class.0);
            assert!(!wide128_sampled(&bind(0, class.1)), "{}: not sampled as planes while closed", class.0);
            continue;
        }
        on_gc7000(|| {
            let m = upload(class);
            let p = program(class.3, class.4, false);
            let s = draw(p);
            let slots = written_slots(&s);
            assert!(!slots.is_empty(), "{}: the vertex stage's slot was written", class.0);
            if !stored_as_planes(class, &s, slots[0]) {
                return;
            }
            assert_eq!(slots.len(), 2, "{}: the unit and its companion, {slots:?}", class.0);
            assert_eq!(slots[1], slots[0] + 1, "{}: the companion right after the vertex stage's slot", class.0);
            let (sense, planes) = wide128_draw_state(p).expect("the draw asked");
            assert_eq!(sense, [1, 0], "{}", class.0);
            assert_eq!(planes[0], Wide128Planes::NONE.with(0, 1), "{}: the vertex stage's key", class.0);
            assert!(planes[1].is_empty(), "{}: the fragment stage reads nothing", class.0);
            check_pair(&format!("{} in the vertex stage", class.0), class, &s, slots[0], slots[1], &m);
        });
    }
}

/// The same texture sampled in both stages of one draw: four descriptors,
/// the fragment stage's at 0 and 1 and the vertex stage's after them, each
/// stage keyed by its own TEX_ID, and both stages' planes the one copy.
#[test]
fn one_texture_in_both_stages_shares_one_copy() {
    for class in &WIDE {
        if !open(class.1) {
            eprintln!("{}: not in GC_SAMPLED_TEXFMTS; the both-stage draw is not exercised (wide128-flip)", class.0);
            continue;
        }
        on_gc7000(|| {
            let m = upload(class);
            let p = program(class.3, class.4, true);
            let s = draw(p);
            let slots = written_slots(&s);
            if !stored_as_planes(class, &s, *slots.first().expect("a descriptor")) {
                return;
            }
            assert_eq!(slots, [0, 1, 2, 3], "{}: fragment unit and companion, vertex unit and companion", class.0);
            let (_, planes) = wide128_draw_state(p).expect("the draw asked");
            assert_eq!(planes, [Wide128Planes::NONE.with(0, 1); 2], "{}: both stages keyed", class.0);
            let f = check_pair(&format!("{} fragment", class.0), class, &s, 0, 1, &m);
            let v = check_pair(&format!("{} vertex", class.0), class, &s, 2, 3, &m);
            assert_eq!(f, v, "{}: one tiled copy for both stages", class.0);
        });
    }
}
