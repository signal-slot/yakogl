// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 4.2 samples the ES 3.0 storage classes (`formats-v3d`): every
//! class the GL core stores natively on the V3D (`backend::V3D_SAMPLED_TEXFMTS`)
//! is a TMU texture type and a return size as Mesa's V3D 4.2 format table
//! has it for the pipe format of the same memory layout
//! (gallium/drivers/v3d/v3dx_format_table.c, Mesa 26.2.1), and the compiler
//! reads an integer sampler's result raw.
//!
//! Pinned here:
//! * the texture shader state record of a binding of each class
//!   (`v3d::texture::state`), packed by the real `driver/src/v3d/v3d_tex.c`
//!   (built by this test, as tests/v3d_depth_storage.rs builds it), decoded
//!   from its bits (v3d_packet.xml TEXTURE_SHADER_STATE, V3D 4.2: Texture
//!   type 100..106, Swizzle R/G/B/A 108..119, sRGB bit 3); and the row of
//!   the C format table for the class (`v3d_tex_format_get`), which
//!   tools/v3d/tex_check.c compares with Mesa's own table: the Rust row, the
//!   C row and Mesa's are one;
//! * the return size: the classes of 32-bit channels (R32F, RGBA32F,
//!   RGBA32UI, RGBA32I) need the 32-bit TMU output, the GL core keys the
//!   program for it (`backend::gles_texfmt_return_32`, LodKey::ret32), and
//!   a binding read at the other output is refused by name;
//! * on the QPU interpreter (`shaderc::online::v3d::sim`, the one
//!   tests/v3d_sim_mesa.rs validates on Mesa's code): an `isampler2D` /
//!   `usampler2D` look-up of a 16-bit-output texture reads each channel's
//!   16 bits sign- or zero-extended (Mesa's nir_lower_tex_packing_16), one
//!   of a 32-bit-output texture reads the words as they are, and an RGBA32F
//!   look-up keyed 32-bit reads its floats whole.
//!
//! The filterability rule (an integer or 32-bit float texture is complete
//! only with NEAREST filters, ES 3.0 3.8.13) is the GL core's
//! (`es3_formats::Es3Format::incomplete_with`); the V3D does not filter
//! 32-bit floats either (Mesa's v3d_screen.c sets no texture_float_linear).
//!
//! `gl_rig` (below) drives the GL API over the real V3D back end, with the job
//! builder faked: RGBA16F, R16F and R32F render targets are complete and read
//! back a clear, and RGB565, RGBA16F and R16F textures reach the TMU.
//!
//! tools/check.sh runs this file in the V3D_CONFIG row. Compiles to nothing
//! under report-es2 (no ES 3.0 storage class is used there).

#![cfg(all(
    feature = "host-test",
    feature = "gpu-v3d",
    feature = "shaderc-online",
    not(feature = "report-es2"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use yakogl::backend::*;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Texture, LANES};
use yakogl::shaderc::online::{lower, sema};
use yakogl::shaderc::{set_compile_lod_key, LodKey};
use yakogl::v3d::ffi::*;
use yakogl::v3d::texture;

/* ---- the real layout and packers, built from driver/src/v3d/v3d_tex.c ------ */

extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// `driver/src/v3d/v3d_tex.c` and the Mesa tiling it includes, as a shared
/// object of this test's own, reached through the handle only (RTLD_LOCAL).
fn lib() -> *mut c_void {
    static L: OnceLock<usize> = OnceLock::new();
    *L.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_formats_v3d");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let so = dir.join(format!("v3d_tex-{}.so", std::process::id()));
        let st = Command::new("cc")
            .current_dir(&root)
            .env("TMPDIR", &dir)
            .args(["-std=c11", "-O1", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
            .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated"])
            .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
            .arg("-Ithird_party/mesa-26.2.1/include")
            .args(["driver/src/v3d/v3d_tex.c", "driver/src/v3d/v3d_tiling_mesa.c", "-lm", "-o"])
            .arg(&so)
            .status()
            .expect("cc runs");
        assert!(st.success(), "driver/src/v3d/v3d_tex.c builds");
        let path = CString::new(so.to_str().expect("a utf-8 path")).unwrap();
        // SAFETY: a path of a library this test just built; RTLD_NOW.
        let h = unsafe { dlopen(path.as_ptr(), 2) };
        assert!(!h.is_null(), "the layout code loads");
        let _ = std::fs::remove_file(&so);
        h as usize
    }) as *mut c_void
}

fn sym(name: &str) -> *mut c_void {
    let c = CString::new(name).unwrap();
    // SAFETY: a handle `lib` made and a NUL-terminated name.
    let p = unsafe { dlsym(lib(), c.as_ptr()) };
    assert!(!p.is_null(), "{name} in v3d_tex.c");
    p
}

/// The real packer: the 24 bytes of TEXTURE_SHADER_STATE, or None when it
/// refuses the record (a type no row of its table has, a field too wide).
fn pack(s: &V3dTexState) -> Option<[u8; V3D_TEX_STATE_BYTES]> {
    // SAFETY: the C function's signature (v3d_tex.h).
    let f: unsafe extern "C" fn(*mut u8, *const V3dTexState) -> bool =
        unsafe { std::mem::transmute(sym("v3d_tex_pack_state")) };
    let mut out = [0u8; V3D_TEX_STATE_BYTES];
    // SAFETY: 24 writable bytes and a live state.
    unsafe { f(out.as_mut_ptr(), s) }.then_some(out)
}

/// `struct v3d_tex_format` (v3d_tex.h).
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct CFormat {
    type_: u32,
    cpp: u32,
    swizzle: [u8; 4],
    return_size: u32,
}

/// The C table's row named `name` (`v3d_tex_format_name`), or None when the
/// table has the name but refuses it; panics on a name it does not have.
fn c_row(name: &str) -> Option<CFormat> {
    // SAFETY: the C functions' signatures (v3d_tex.h).
    let name_of: unsafe extern "C" fn(c_int) -> *const c_char =
        unsafe { std::mem::transmute(sym("v3d_tex_format_name")) };
    // SAFETY: as above.
    let get: unsafe extern "C" fn(c_int, *mut CFormat) -> bool =
        unsafe { std::mem::transmute(sym("v3d_tex_format_get")) };
    for id in 0..64 {
        // SAFETY: a static NUL-terminated name for every id.
        let n = unsafe { CStr::from_ptr(name_of(id)) }.to_str().unwrap();
        if n == "unknown" {
            break;
        }
        if n == name {
            let mut f = CFormat::default();
            // SAFETY: a live out-parameter.
            return unsafe { get(id, &mut f) }.then_some(f);
        }
    }
    panic!("v3d_tex.c has no format named {name}")
}

/// The fields of a packed record (v3d_packet.xml, Texture Shader State,
/// V3D 4.2): (Texture type, [Swizzle R, G, B, A], sRGB).
fn decode(rec: &[u8; V3D_TEX_STATE_BYTES]) -> (u32, [u8; 4], bool) {
    let mut lo = [0u8; 16];
    lo.copy_from_slice(&rec[..16]);
    let x = u128::from_le_bytes(lo);
    let field = |start: u32, size: u32| ((x >> start) & ((1u128 << size) - 1)) as u32;
    (
        field(100, 7),
        [field(108, 3) as u8, field(111, 3) as u8, field(114, 3) as u8, field(117, 3) as u8],
        field(3, 1) != 0,
    )
}

/* ---- the rows --------------------------------------------------------------- */

const XYZW: [u8; 4] = [2, 3, 4, 5];
const XYZ1: [u8; 4] = [2, 3, 4, 1];

/// (class, its name in v3d_tex.c, TEXTURE_DATA_FORMAT (v3d_packet.xml),
/// swizzle (v3d_translate_pipe_swizzle of Mesa's), bytes a texel, return
/// size 32). Mesa's rows, v3dx_format_table.c (Mesa 26.2.1):
type Row = (u8, &'static str, u32, [u8; 4], u32, bool);
const ROWS: [Row; 16] = [
    (GLES_TEXFMT_RGBA8, "RGBA8", 4, XYZW, 4, false), /* :65 R8G8B8A8_UNORM */
    (GLES_TEXFMT_RGB565, "RGB565", 6, XYZ1, 2, false), /* :81 B5G6R5_UNORM */
    (GLES_TEXFMT_R16F, "R16F", 16, [2, 0, 0, 1], 2, false), /* :90 R16_FLOAT */
    (GLES_TEXFMT_R32F, "R32F", 29, [2, 0, 0, 1], 4, true), /* :91 R32_FLOAT */
    (GLES_TEXFMT_RGBA16F, "RGBA16F", 18, XYZW, 8, false), /* :100 R16G16B16A16_FLOAT */
    (GLES_TEXFMT_RGBA32F, "RGBA32F", 31, XYZW, 16, true), /* :101 R32G32B32A32_FLOAT */
    (GLES_TEXFMT_R11G11B10F, "R11G11B10F", 19, XYZ1, 4, false), /* :146 R11G11B10_FLOAT */
    (GLES_TEXFMT_RGB9E5, "RGB9E5", 20, XYZ1, 4, false), /* :147 R9G9B9E5_FLOAT */
    (GLES_TEXFMT_RGB10A2UI, "RGB10A2UI", 114, XYZW, 4, false), /* :75 R10G10B10A2_UINT */
    (GLES_TEXFMT_RGBA8_SNORM, "RGBA8_SNORM", 5, XYZW, 4, false), /* :69 R8G8B8A8_SNORM */
    (GLES_TEXFMT_RGBA8UI, "RGBA8UI", 101, XYZW, 4, false), /* :123 R8G8B8A8_UINT */
    (GLES_TEXFMT_RGBA8I, "RGBA8I", 100, XYZW, 4, false), /* :122 R8G8B8A8_SINT */
    (GLES_TEXFMT_RGBA16UI, "RGBA16UI", 107, XYZW, 8, false), /* :130 R16G16B16A16_UINT */
    (GLES_TEXFMT_RGBA16I, "RGBA16I", 106, XYZW, 8, false), /* :129 R16G16B16A16_SINT */
    (GLES_TEXFMT_RGBA32UI, "RGBA32UI", 113, XYZW, 16, true), /* :137 R32G32B32A32_UINT */
    (GLES_TEXFMT_RGBA32I, "RGBA32I", 112, XYZW, 16, true), /* :136 R32G32B32A32_SINT */
];

fn bind(format: u8, w: u32, h: u32) -> GlesTextureBind {
    GlesTextureBind {
        width: w,
        height: h,
        stride_texels: (w + 1) & !1,
        layers: 1,
        mip_levels: 1,
        format,
        ..GlesTextureBind::ZERO
    }
}

/// Every class of the mask is a row above, and the rows are the mask: what
/// the GL core stores natively on the V3D, its TMU samples. RGB10A2 (kept
/// in RGBA8 here, as on the PowerVR) and the ETC2 formats are refused by
/// name.
#[test]
fn the_sampled_mask_is_the_rows() {
    let rows = ROWS.iter().fold(0u32, |m, r| m | gles_texfmt_bit(r.0));
    let narrow = [GLES_TEXFMT_L8, GLES_TEXFMT_A8, GLES_TEXFMT_LA8, GLES_TEXFMT_SRGB8_A8]
        .iter()
        .fold(0u32, |m, &f| m | gles_texfmt_bit(f));
    assert_eq!(V3D_SAMPLED_TEXFMTS, rows | narrow, "the mask is the rows and the ES 2.0 narrow classes");
    for f in [GLES_TEXFMT_RGB10A2, GLES_TEXFMT_ETC2_RGB, GLES_TEXFMT_ETC2_RGBA, GLES_TEXFMT_ETC2_RGB_PT] {
        let b = bind(f, 4, 4);
        let r = texture::refusal_for(&b, false).expect("refused");
        assert!(r.contains("does not sample"), "format {f}: {r}");
    }
}

/// Each class: the Rust row (`tmu_format`, `return_32`, `texel_bytes`) is
/// the C table's row (v3d_tex.c, which tex_check compares with Mesa's) and
/// Mesa's (ROWS); the binding lays out at its own size and is sampled only
/// at its own TMU output width.
#[test]
fn each_class_is_mesas_row_in_rust_and_in_c() {
    for (f, name, type_, swizzle, bytes, ret32) in ROWS {
        let b = bind(f, 8, 4);
        assert_eq!(texture::tmu_format(&b), (type_, swizzle), "{name}");
        assert_eq!((texture::texel_bytes(&b), texture::return_32(&b)), (bytes, ret32), "{name}");
        assert_eq!(gles_texfmt_return_32(f), ret32, "{name}: the GL core's key bit");
        let c = c_row(name).unwrap_or_else(|| panic!("v3d_tex.c refuses {name}"));
        assert_eq!(
            c,
            CFormat { type_, cpp: bytes, swizzle, return_size: if ret32 { 32 } else { 16 } },
            "{name}: v3d_tex.c's row"
        );
        assert_eq!(texture::refusal_for(&b, ret32), None, "{name}");
        let other = texture::refusal_for(&b, !ret32).expect("the other output width is refused");
        assert!(other.contains("look-up compiled for the"), "{name}: {other}");
        assert_eq!(texture::layout(&b).expect("lays out").cpp, bytes, "{name}: laid out at its own size");
    }
    /* RGB10A2's row is there (Mesa's :71), though the class is not sampled */
    assert_eq!(texture::tmu_format(&bind(GLES_TEXFMT_RGB10A2, 4, 4)), (9, XYZW));
    assert_eq!(c_row("RGB10A2").map(|c| c.type_), Some(9));
}

/// The texture shader state record of a binding of each class, packed by
/// v3d_tex.c's packer (Mesa's V3D42 TEXTURE_SHADER_STATE packer): its type
/// and swizzle fields are the row's, the sRGB bit is set for the sRGB class
/// only, and it packs at all (the packer refuses a type no row has).
#[test]
fn each_classs_record_carries_its_type_and_swizzle() {
    let mut rows: Vec<(u8, u32, [u8; 4], bool)> = ROWS.iter().map(|r| (r.0, r.2, r.3, false)).collect();
    rows.push((GLES_TEXFMT_SRGB8_A8, 4, XYZW, true));
    for (f, type_, swizzle, srgb) in rows {
        for (w, h) in [(4, 4), (64, 32), (13, 7)] {
            let b = bind(f, w, h);
            let l = texture::layout(&b).expect("lays out");
            let st = texture::state(&b, &l, 0x0010_0000);
            let rec = pack(&st).unwrap_or_else(|| panic!("format {f} {w}x{h}: the packer refused {st:?}"));
            assert_eq!(decode(&rec), (type_, swizzle, srgb), "format {f} {w}x{h}");
        }
    }
    /* the packer refuses a type no row of its table has (R8 SNORM, 1) */
    let b = bind(GLES_TEXFMT_RGBA8, 4, 4);
    let l = texture::layout(&b).unwrap();
    assert!(pack(&V3dTexState { type_: 1, ..texture::state(&b, &l, 0x0010_0000) }).is_none());
}

/// A depth texture's R32F binding keeps its two swizzles (ES 3.0 red, ES 2.0
/// luminance) at the 32-bit output, as before this item.
#[test]
fn the_depth_binding_is_unchanged() {
    let es3 = bind(GLES_TEXFMT_R32F, 4, 4);
    let es2 = GlesTextureBind { depth_luminance: 1, ..es3 };
    assert_eq!(texture::tmu_format(&es3), (29, [2, 0, 0, 1]));
    assert_eq!(texture::tmu_format(&es2), (29, [2, 2, 2, 1]));
}

/* ---- the compiler on the interpreter ----------------------------------------- */

/// The compiler keeps its output in one arena, and the look-up key is one
/// global: one compile at a time.
static COMPILING: Mutex<()> = Mutex::new(());

const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";

/// What the fragment shader `fs` (one look-up of texture unit 0 at (0.25,
/// 0.25), a NEAREST 2x2 texture whose texel (0, 0) is `texel`) wrote to the
/// TLB for lane 0, compiled with `ret32` as the binding's key bit, and the
/// P0/P1 words of its stream. `integer`: the texture is of an integer type.
fn run(fs: &str, ret32: bool, texel: [f32; 4], integer: bool) -> (Vec<u32>, Vec<(u32, u32)>) {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let old = set_compile_lod_key(LodKey { ret32: u32::from(ret32), ..LodKey::NONE });
    struct Restore(LodKey);
    impl Drop for Restore {
        fn drop(&mut self) {
            set_compile_lod_key(self.0);
        }
    }
    let _restore = Restore(old);
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(VS).expect("arena");
    let fs = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).map_err(|e| e.msg.to_string()).expect("the FS lowers");
    let p = compile_program(&v.module, &f.module, Prim::Triangles, Target { vpm_size: 65536 }, &a)
        .unwrap_or_else(|e| panic!("the program compiles: {e:?}"));
    let cfg: Vec<(u32, u32)> =
        p.fs.code.uniforms.iter().filter(|u| matches!(u.contents, 40 | 41)).map(|u| (u.contents, u.data)).collect();
    let uniforms: Vec<u32> =
        p.fs.code
            .uniforms
            .iter()
            .map(|u| match u.contents {
                0 | 40 | 41 => u.data,
                c => panic!("uniform contents {c} not modelled by this test"),
            })
            .collect();
    let inputs = FsInputs {
        w: [1.0f32.to_bits(); LANES],
        w_centroid: [1.0f32.to_bits(); LANES],
        z: [0; LANES],
        x: [0; LANES],
        y: [0; LANES],
        back_facing: [false; LANES],
        varyings: Vec::new(),
    };
    let other = [7.0f32, 7.0, 7.0, 7.0];
    let tex = Texture::flat(2, 2, vec![texel, other, other, other]);
    let env =
        Env { threads: p.fs.threads, textures: vec![tex], integer_textures: u32::from(integer), ..Env::default() };
    let o = sim::run(p.fs.code.words, &uniforms, &Inputs::Fragment(Box::new(inputs)), &env)
        .unwrap_or_else(|e| panic!("the fragment program runs: {e}"));
    (o.tlb.iter().map(|w| w.value[0].expect("a defined colour word")).collect(), cfg)
}

const ISAMPLER: &str = "#version 300 es\nprecision highp float;\nprecision highp int;\n\
    uniform highp isampler2D s;\nout ivec4 o;\nvoid main() { o = texture(s, vec2(0.25, 0.25)); }\n";
const USAMPLER: &str = "#version 300 es\nprecision highp float;\nprecision highp int;\n\
    uniform highp usampler2D s;\nout uvec4 o;\nvoid main() { o = texture(s, vec2(0.25, 0.25)); }\n";

/// RGBA8I / RGBA16I at the 16-bit output: two channels a word, each read
/// back sign-extended from its 16 bits; the TLB gets the four integers as
/// 32-bit words (the integer output). Two result words, P1 without the
/// 32-bit bit. Revert check: with the f16 unpack of a float sampler the
/// words are float bits, and with a zero extension -5 reads 65531.
#[test]
fn an_isampler_reads_16_bit_channels_sign_extended() {
    let texel = [-5.0, 7.0, -32768.0, 32767.0];
    let (tlb, cfg) = run(ISAMPLER, false, texel, true);
    assert_eq!(cfg, [(40, 0x3), (41, 0x4)], "two words, 16-bit output");
    assert_eq!(tlb, [-5i32 as u32, 7, -32768i32 as u32, 32767]);
}

/// RGBA8UI / RGBA16UI / RGB10A2UI at the 16-bit output: zero-extended.
#[test]
fn a_usampler_reads_16_bit_channels_zero_extended() {
    let texel = [65535.0, 0.0, 1.0, 1023.0];
    let (tlb, cfg) = run(USAMPLER, false, texel, true);
    assert_eq!(cfg, [(40, 0x3), (41, 0x4)]);
    assert_eq!(tlb, [65535, 0, 1, 1023]);
}

/// RGBA32I / RGBA32UI, keyed for the 32-bit output (the GL core sets the
/// key bit from `gles_texfmt_return_32`): four words, P1's "Output Type
/// 32-bit", and each channel its word as it is.
#[test]
fn a_32_bit_integer_texture_reads_its_words_whole() {
    let (tlb, cfg) = run(ISAMPLER, true, [-2.0, 2_000_000.0, -70000.0, 1.0], true);
    assert_eq!(cfg, [(40, 0xf), (41, 0x5)], "four words, 32-bit output");
    assert_eq!(tlb, [-2i32 as u32, 2_000_000, -70000i32 as u32, 1]);
    let (tlb, _) = run(USAMPLER, true, [3_000_000_000.0, 70000.0, 0.0, 1.0], true);
    assert_eq!(tlb, [3_000_000_000, 70000, 0, 1]);
}

/// RGBA32F keyed 32-bit: the channels are the words, the TLB's f16 output
/// packs them as any float colour; at the 16-bit output (RGBA16F) the same
/// shader reads f16 halves, as before.
#[test]
fn an_rgba32f_texture_reads_32_bit_floats() {
    const FS: &str = "#version 300 es\nprecision highp float;\nuniform highp sampler2D s;\nout vec4 o;\n\
        void main() { o = texture(s, vec2(0.25, 0.25)); }\n";
    let texel = [1.5, -2.0, 1024.0, 0.25];
    let h = |x: f32| u32::from(sim::f32_to_half(x));
    let want = [h(1.5) | h(-2.0) << 16, h(1024.0) | h(0.25) << 16];
    let (tlb32, cfg32) = run(FS, true, texel, false);
    assert_eq!(cfg32, [(40, 0xf), (41, 0x5)]);
    assert_eq!(tlb32, want);
    let (tlb16, cfg16) = run(FS, false, texel, false);
    assert_eq!(cfg16, [(40, 0x3), (41, 0x4)]);
    assert_eq!(tlb16, want);
}

/* ---- the GL API over the V3D back end ---------------------------------------
 * The real back end (`yakogl_backend_v3d`) behind the GL core, the job
 * builder (v3d_job.c) stood in for by fakes, as tests/depth_resolve_v3d.rs
 * does, and a "GPU" that stores a typed clear into the raster target. */

mod gl_rig {
    #![allow(clippy::too_many_arguments)]
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::{MutexGuard, Once};
    use yakogl::gl::consts::*;
    use yakogl::gl::context::{self, GlContext};
    use yakogl::gl::draw::glDrawArrays;
    use yakogl::gl::framebuffer::*;
    use yakogl::gl::program::*;
    use yakogl::gl::shader::*;
    use yakogl::gl::state::*;
    use yakogl::gl::texture::*;
    use yakogl::gl::types::*;
    use yakogl::gl::vertex::*;
    use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
    use yakogl::v3d::backend::*;
    use yakogl::v3d::kick::V3dKick;

    /// The trampolines: the library's `extern "C"` texture functions
    /// (src/v3d/ffi.rs), each handed to the real C. `v3d_tex_pack_state` also
    /// records the state it packed, by the record's address, for the checks.
    macro_rules! trampoline {
    ($name:ident ( $($arg:ident : $ty:ty),* ) $(-> $ret:ty)?) => {
        #[no_mangle]
        unsafe extern "C" fn $name($($arg: $ty),*) $(-> $ret)? {
            // SAFETY: the C function's own signature, as src/v3d/ffi.rs declares it.
            let f: unsafe extern "C" fn($($ty),*) $(-> $ret)? =
                unsafe { std::mem::transmute(sym(stringify!($name))) };
            // SAFETY: the library's call, passed on.
            unsafe { f($($arg),*) }
        }
    };
}

    /* The signatures are src/v3d/ffi.rs's. Kept as written: a macro call wider
     * than the width is wrapped by some rustfmt releases and left by others, so
     * the gate's `cargo fmt --check` must not depend on the host's release. */
    #[rustfmt::skip]
mod trampolines {
    use super::*;

    trampoline!(v3d_tex_layout(width: u32, height: u32, levels: u32, cpp: u32, l: *mut V3dTexLayout) -> bool);
    trampoline!(v3d_tex_layout_msaa(width: u32, height: u32, cpp: u32, l: *mut V3dTexLayout) -> bool);
    trampoline!(v3d_tex_store_level(image: *mut u8, l: *const V3dTexLayout, level: u32, src: *const u8, stride: u32));
    trampoline!(v3d_tex_tfu_pack(l: *const V3dTexLayout, ttype: u32, src: u32, stride: u32, dst: u32,
                                 j: *mut V3dTexTfuJob) -> bool);
    trampoline!(v3d_tex_tfu_pack_mipmap(l: *const V3dTexLayout, ttype: u32, image: u32, j: *mut V3dTexTfuJob) -> bool);
    trampoline!(v3d_tex_layer_stride(l: *const V3dTexLayout) -> u32);
    trampoline!(v3d_tex_layers_size(l: *const V3dTexLayout, layers: u32, out: *mut u32) -> bool);
    trampoline!(v3d_tex_store_box(image: *mut u8, l: *const V3dTexLayout, level: u32, x: u32, y: u32, w: u32, h: u32,
                                  src: *const u8, stride: u32) -> bool);
    trampoline!(v3d_tex_load_box(image: *const u8, l: *const V3dTexLayout, level: u32, x: u32, y: u32, w: u32, h: u32,
                                 dst: *mut u8, stride: u32) -> bool);
    trampoline!(v3d_tex_state_from_layout(s: *mut V3dTexState, l: *const V3dTexLayout));
    trampoline!(v3d_tex_pack_sampler(out: *mut u8, s: *const V3dSampler) -> bool);
}

    #[no_mangle]
    unsafe extern "C" fn v3d_tex_pack_state(out: *mut u8, s: *const V3dTexState) -> bool {
        // SAFETY: the C function's signature.
        let f: unsafe extern "C" fn(*mut u8, *const V3dTexState) -> bool =
            unsafe { std::mem::transmute(sym("v3d_tex_pack_state")) };
        // SAFETY: the library's call, passed on.
        unsafe { f(out, s) }
    }

    /* ---- the job builder, stood in for (driver/src/v3d/v3d_job.c) -------------- */

    #[derive(Clone, Debug)]
    struct SeenJob {
        target: V3dJobTarget,
        clear: Option<V3dJobClear>,
        /// The FS texture records of each draw, in order.
        tmu: Vec<Vec<V3dJobTmu>>,
    }

    static JOBS: Mutex<Vec<SeenJob>> = Mutex::new(Vec::new());

    fn jobs() -> MutexGuard<'static, Vec<SeenJob>> {
        JOBS.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_begin(
        j: *mut V3dJob,
        t: *const V3dJobTarget,
        bcl: *mut V3dCl,
        rcl: *mut V3dCl,
        ind: *mut V3dCl,
    ) -> bool {
        // SAFETY: the backend passes its live job, target and lists.
        let (j, t) = unsafe { (&mut *j, *t) };
        *j = V3dJob { bcl, rcl, ind, t, draws: 0, defaults: 0, broken: false, pack: V3dJobPack::ZERO };
        // SAFETY: as above.
        unsafe {
            (*bcl).used = 16;
            (*ind).used = 256;
        }
        jobs().push(SeenJob { target: t, clear: None, tmu: Vec::new() });
        t.width != 0 && t.height != 0
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
        // SAFETY: the backend passes its live job and a draw whose pointers live for the call.
        let (j, d) = unsafe { (&mut *j, &*d) };
        let tmu = if d.tmu.is_null() || d.num_tmu == 0 {
            Vec::new()
        } else {
            // SAFETY: `num_tmu` records, live for the call.
            unsafe { std::slice::from_raw_parts(d.tmu, d.num_tmu as usize) }.to_vec()
        };
        j.draws += 1;
        // SAFETY: the job's BCL is live.
        unsafe { (*j.bcl).used += 120 };
        if let Some(job) = jobs().last_mut() {
            job.tmu.push(tmu);
        }
        V3D_JOB_OK
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_end(
        j: *mut V3dJob,
        c: *const V3dJobClear,
        _tile_list_start: *mut u32,
        _tile_list_end: *mut u32,
    ) -> bool {
        // SAFETY: the backend passes its live job and clear.
        let (j, c) = unsafe { (&mut *j, *c) };
        // SAFETY: the job's lists are live.
        unsafe {
            (*j.bcl).used += 1;
            (*j.rcl).used = 200;
        }
        if let Some(job) = jobs().last_mut() {
            job.clear = Some(c);
        }
        true
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling(width: u32, height: u32, t: *mut V3dTiling) {
        // SAFETY: the backend's out-parameter, passed on.
        unsafe { v3d_frame_tiling_bpp(width, height, 1, false, 0, t) }
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling_rts(width: u32, height: u32, rts: u32, t: *mut V3dTiling) {
        // SAFETY: the backend's out-parameter, passed on.
        unsafe { v3d_frame_tiling_bpp(width, height, rts, false, 0, t) }
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling_ms(width: u32, height: u32, rts: u32, msaa: bool, t: *mut V3dTiling) {
        // SAFETY: the backend's out-parameter, passed on.
        unsafe { v3d_frame_tiling_bpp(width, height, rts, msaa, 0, t) }
    }

    /// The tile-size table of v3d_frame.c (Mesa's v3d_choose_tile_size), as
    /// the backend's unit tests fake it.
    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling_bpp(
        width: u32,
        height: u32,
        rts: u32,
        msaa: bool,
        max_bpp: u32,
        t: *mut V3dTiling,
    ) {
        const SIZES: [(u32, u32); 7] = [(64, 64), (64, 32), (32, 32), (32, 16), (16, 16), (16, 8), (8, 8)];
        let idx = match rts {
            0 | 1 => 0,
            2 => 1,
            _ => 2,
        } + if msaa { 2 } else { 0 }
            + max_bpp.min(2) as usize;
        let (tw, th) = SIZES[idx];
        let tiles = V3dTiling {
            tile_w: tw,
            tile_h: th,
            tiles_x: width.div_ceil(tw),
            tiles_y: height.div_ceil(th),
            supertile_w: 1,
            supertile_h: 1,
            frame_w_supertiles: width.div_ceil(tw),
            frame_h_supertiles: height.div_ceil(th),
        };
        // SAFETY: the backend's out-parameter.
        unsafe { *t = tiles };
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_alloc_sizes(
        t: *const V3dTiling,
        _draws: u32,
        _page: u32,
        ta: *mut u32,
        ts: *mut u32,
    ) {
        // SAFETY: the backend's parameters.
        unsafe {
            let n = (*t).tiles_x * (*t).tiles_y;
            *ta = n * 128 + 8192;
            *ts = n * 256;
        }
    }

    /* ---- the platform ------------------------------------------------------------ */

    /// GPU addresses are 32-bit (the V3D's MMU), from here up.
    const GPU_BASE: u64 = 0x0100_0000;

    struct Alloc {
        /// u64 backing: every u32 view of it is aligned.
        mem: Box<[u64]>,
        size: usize,
    }

    #[derive(Default)]
    struct Platform {
        next: u64,
        allocs: BTreeMap<u64, Alloc>,
        kicks: Vec<V3dKick>,
        log: Vec<String>,
    }

    static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

    fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
        let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
        f(g.get_or_insert_with(Platform::default))
    }

    /// The CPU view of `n` bytes at GPU address `gpu`.
    fn platform_cpu(p: &mut Platform, gpu: u64, n: usize) -> *mut u8 {
        let (base, a) = p.allocs.range_mut(..=gpu).next_back().expect("an allocation below the address");
        let off = (gpu - base) as usize;
        assert!(off + n <= a.size, "0x{gpu:x}+{n} is past its allocation");
        // SAFETY: inside the allocation.
        unsafe { (a.mem.as_mut_ptr() as *mut u8).add(off) }
    }

    unsafe extern "C" fn p_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
        with_platform(|p| {
            if size == 0 || size > 256 << 20 {
                return -12;
            }
            let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
            let gpu = GPU_BASE + p.next;
            p.next += (size as u64).div_ceil(4096) * 4096;
            let cpu = mem.as_mut_ptr() as *mut c_void;
            // SAFETY: the caller passes a writable descriptor.
            unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
            p.allocs.insert(gpu, Alloc { mem, size });
            0
        })
    }

    unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
        // SAFETY: the caller passes its descriptor.
        let m = unsafe { *mem };
        if !m.impl_.is_null() {
            with_platform(|p| p.allocs.remove(&m.gpu));
        }
        0
    }

    unsafe extern "C" fn p_gpu_write(mem: *mut OsalGpuMem, off: usize, src: *const c_void, n: usize) -> i32 {
        // SAFETY: the caller passes its descriptor.
        let m = unsafe { *mem };
        if m.cpu.is_null() || off.checked_add(n).is_none_or(|e| e > m.size) {
            return -22;
        }
        // SAFETY: inside the allocation (checked above).
        unsafe { core::ptr::copy_nonoverlapping(src as *const u8, (m.cpu as *mut u8).add(off), n) };
        0
    }

    unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

    /// The "GPU": every job completes inside its submit (as src/osal/linux.c),
    /// and a job that stores a typed colour target with a clear leaves the
    /// clear's texel (the tile buffer words `V3dJobClear::clear_rt[0]`, in the
    /// output format the target names) in every pixel of the raster store.
    unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
        if len != size_of::<V3dKick>() {
            return -22;
        }
        // SAFETY: the backend's live descriptor of that size.
        let k = unsafe { *(kick as *const V3dKick) };
        let fence = with_platform(|p| {
            p.kicks.push(k);
            p.kicks.len()
        });
        let job = jobs().get(fence - 1).cloned().expect("the job this kick runs was begun and ended");
        let (t, c) = (job.target, job.clear.expect("the job was ended"));
        if t.typed && !c.load_rt0 && t.address != 0 {
            let w = c.clear_rt[0];
            let texel: Vec<u8> = match t.rt_format[0] {
                V3D_OUTPUT_IMAGE_FORMAT_RGBA16F => w[..2].iter().flat_map(|x| x.to_le_bytes()).collect(),
                V3D_OUTPUT_IMAGE_FORMAT_R16F => w[0].to_le_bytes()[..2].to_vec(),
                V3D_OUTPUT_IMAGE_FORMAT_R32F => w[0].to_le_bytes().to_vec(),
                _ => Vec::new(),
            };
            if !texel.is_empty() {
                with_platform(|p| {
                    let row = (t.stride as usize).max(t.width as usize * texel.len());
                    let base = platform_cpu(p, u64::from(t.address), row * t.height as usize);
                    for y in 0..t.height as usize {
                        for x in 0..t.width as usize {
                            // SAFETY: inside the span platform_cpu checked.
                            unsafe {
                                core::ptr::copy_nonoverlapping(
                                    texel.as_ptr(),
                                    base.add(y * row + x * texel.len()),
                                    texel.len(),
                                )
                            };
                        }
                    }
                });
            }
        }
        fence as i64
    }

    unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
        0
    }

    unsafe extern "C" fn p_completed() -> i64 {
        with_platform(|p| p.kicks.len() as i64)
    }

    unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
        use yakogl::osal::{
            OSAL_QUERY_V3D_BIN_POOL, OSAL_QUERY_V3D_CPU_OF_GPU, OSAL_QUERY_V3D_JOB_OPEN, OSAL_QUERY_V3D_VPM_BYTES,
        };
        if what == OSAL_QUERY_V3D_VPM_BYTES && out_sz == 4 {
            // SAFETY: a u32 out-parameter.
            unsafe { *(out as *mut u32) = 65536 };
            return 0;
        }
        if what == OSAL_QUERY_V3D_CPU_OF_GPU && out_sz == size_of::<OsalV3dCpuOf>() {
            // SAFETY: the backend's in/out struct of that size.
            let q = unsafe { &mut *(out as *mut OsalV3dCpuOf) };
            return with_platform(|p| match p.allocs.range(..=q.gpu).next_back() {
                Some((base, a)) if q.gpu + q.bytes <= base + a.size as u64 => {
                    // SAFETY: inside the allocation.
                    q.cpu = unsafe { (a.mem.as_ptr() as *mut u8).add((q.gpu - base) as usize) } as *mut c_void;
                    0
                }
                _ => -22,
            });
        }
        if what == OSAL_QUERY_V3D_JOB_OPEN && out_sz == 4 {
            return 0;
        }
        if what == OSAL_QUERY_V3D_BIN_POOL {
            return -61; /* ENODATA, as zephyr_v3d.c answers before the first job */
        }
        -22 /* no TFU, the default budgets, no clock */
    }

    /// The library logs every line as `log("%s", line)` (`osal::log_str`), so a
    /// two-pointer function takes it; on x86-64 and AArch64 Linux a variadic call
    /// passes those two arguments as a plain one does.
    unsafe extern "C" fn p_log2(_fmt: *const c_char, line: *const c_char) {
        // SAFETY: the library passes a NUL-terminated line.
        let s = unsafe { CStr::from_ptr(line) }.to_string_lossy().into_owned();
        with_platform(|p| p.log.push(s));
    }

    type LogFn = unsafe extern "C" fn(*const c_char, ...);

    unsafe extern "C" fn p_init(_platform_arg: *mut c_void) -> i32 {
        0
    }

    /// The platform's table (made once, at run time: the log hook is a cast).
    fn osal() -> &'static OsalOps {
        // SAFETY: see p_log2: the library only ever calls it with ("%s", line).
        let log = unsafe { core::mem::transmute::<unsafe extern "C" fn(*const c_char, *const c_char), LogFn>(p_log2) };
        Box::leak(Box::new(OsalOps {
            init: Some(p_init),
            shutdown: None,
            gpu_alloc: Some(p_gpu_alloc),
            gpu_free: Some(p_gpu_free),
            mmu_map: None,
            map_phys: None,
            gpu_write: Some(p_gpu_write),
            gpu_read: None,
            cache_flush: Some(p_cache),
            cache_invd: Some(p_cache),
            submit: Some(p_submit),
            wait: Some(p_wait),
            completed: Some(p_completed),
            reg_read: None,
            reg_write: None,
            query: Some(p_query),
            log: Some(log),
            wait_vblank: None,
            present: None,
            lock: None,
            unlock: None,
            thread_id: None,
            display_busy: None,
        }))
    }

    /* ---- the context ------------------------------------------------------------- */

    /// pair of the same shape keeps the test quick.
    const CANVAS: (i32, i32) = (16, 16);

    static LOCK: Mutex<()> = Mutex::new(());
    static ONCE: Once = Once::new();
    static mut CTX: *mut GlContext = core::ptr::null_mut();

    /// The one context and backend of the file, set up once; the tests take
    /// turns.
    fn rig() -> MutexGuard<'static, ()> {
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        ONCE.call_once(|| {
            // SAFETY: statics; the lock serialises every GL call of the file.
            unsafe {
                osal_set_ops(osal());
                yakogl::backend::g_backend = &yakogl_backend_v3d;
                assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
                CTX = context::gl_context_create(CANVAS.0 as u32, CANVAS.1 as u32);
                assert!(!CTX.is_null(), "{:?}", logs());
            }
        });
        // SAFETY: plain GL calls.
        unsafe {
            glFinish();
            while glGetError() != GL_NO_ERROR {}
        }
        g
    }

    fn logs() -> Vec<String> {
        with_platform(|p| p.log.clone())
    }

    /* ---- the pins ---------------------------------------------------------- */

    fn shader(type_: GLenum, src: &str) -> GLuint {
        let c = CString::new(src).unwrap();
        // SAFETY: plain GL calls; the string lives for the call.
        unsafe {
            let s = glCreateShader(type_);
            let srcs = [c.as_ptr()];
            glShaderSource(s, 1, srcs.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, 1, "compile 0x{type_:x}: {src}\n{:?}", logs());
            s
        }
    }

    fn program(vs: &str, fs: &str) -> GLuint {
        let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
        // SAFETY: plain GL calls on valid handles.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, f);
            glBindAttribLocation(p, 0, c"a_position".as_ptr());
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "link failed: {:?}", logs());
            p
        }
    }

    const SIDE: i32 = 8;

    /// A texture of `internal` (`format`/`type_`, no texels) attached to a
    /// framebuffer: the status the GL core gives it, and what a clear to
    /// (0.5, 0.25, 0.75, 1) reads back as RGBA/FLOAT.
    fn clear_and_read(internal: GLenum, format: GLenum, type_: GLenum) -> (GLenum, Vec<f32>) {
        let (mut tex, mut fbo) = (0u32, 0u32);
        let mut px = vec![-1.0f32; (SIDE * SIDE * 4) as usize];
        // SAFETY: plain GL calls with live out pointers and a buffer of SIDE x SIDE RGBA floats.
        let status = unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, SIDE, SIDE, 0, format, type_, core::ptr::null());
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            let status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
            if status == GL_FRAMEBUFFER_COMPLETE {
                glViewport(0, 0, SIDE, SIDE);
                glClearColor(0.5, 0.25, 0.75, 1.0);
                glClear(GL_COLOR_BUFFER_BIT);
                glReadPixels(0, 0, SIDE, SIDE, GL_RGBA, GL_FLOAT, px.as_mut_ptr() as *mut c_void);
            }
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fbo);
            glDeleteTextures(1, &tex);
            status
        };
        (status, px)
    }

    #[test]
    fn rgba16f_target_is_complete_and_clears() {
        let _g = rig();
        let (status, px) = clear_and_read(GL_RGBA16F, GL_RGBA, GL_HALF_FLOAT);
        assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
        assert!(px.chunks(4).all(|p| p == [0.5, 0.25, 0.75, 1.0]), "{:?}", &px[..8]);
    }

    #[test]
    fn r16f_target_is_complete_and_clears() {
        let _g = rig();
        let (status, px) = clear_and_read(GL_R16F, GL_RED, GL_HALF_FLOAT);
        assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
        assert!(px.chunks(4).all(|p| p == [0.5, 0.0, 0.0, 1.0]), "{:?}", &px[..8]);
    }

    #[test]
    fn r32f_target_is_complete_and_clears() {
        let _g = rig();
        let (status, px) = clear_and_read(GL_R32F, GL_RED, GL_FLOAT);
        assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
        assert!(px.chunks(4).all(|p| p == [0.5, 0.0, 0.0, 1.0]), "{:?}", &px[..8]);
    }

    const VS: &str =
        "#version 300 es\nlayout(location = 0) in vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";
    const FS: &str = "#version 300 es\nprecision highp float;\nuniform sampler2D s;\nout vec4 o;\n\
        void main() { o = texture(s, vec2(0.5)); }\n";
    static QUAD: [f32; 18] = [
        -1.0, -1.0, 0.0, 1.0, -1.0, 0.0, -1.0, 1.0, 0.0, //
        -1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0,
    ];

    /// A SIDE x SIDE texture of `internal` filled from `texels`, sampled by
    /// a draw into the canvas: the texture reaches the TMU (a texture record
    /// in the draw) and the back end says nothing about refusing it.
    fn sampled(internal: GLenum, format: GLenum, type_: GLenum, texels: &[u8]) {
        let prog = program(VS, FS);
        let mut tex = 0u32;
        let before = jobs().iter().map(|j| j.tmu.len()).sum::<usize>();
        // SAFETY: plain GL calls; `texels` is a SIDE x SIDE image of the type.
        unsafe {
            glGenTextures(1, &mut tex);
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                internal as GLint,
                SIDE,
                SIDE,
                0,
                format,
                type_,
                texels.as_ptr() as *const c_void,
            );
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glViewport(0, 0, CANVAS.0, CANVAS.1);
            glUseProgram(prog);
            glUniform1i(glGetUniformLocation(prog, c"s".as_ptr()), 0);
            glVertexAttribPointer(0, 3, GL_FLOAT, 0, 12, QUAD.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 6);
            glFinish();
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
        let after: Vec<_> = jobs().iter().flat_map(|j| j.tmu.iter().cloned()).collect();
        assert!(after.len() > before, "the draw reached the job builder: {:?}", logs());
        assert!(after.last().is_some_and(|t| !t.is_empty()), "the draw carries a texture record: {:?}", logs());
        let refused: Vec<_> =
            logs().into_iter().filter(|l| l.contains("does not sample") || l.contains("refus")).collect();
        assert!(refused.is_empty(), "{refused:?}");
        // SAFETY: names made above.
        unsafe {
            glDeleteTextures(1, &tex);
            glDeleteProgram(prog);
        }
    }

    #[test]
    fn an_rgb565_texture_is_sampled() {
        let _g = rig();
        sampled(GL_RGB565, GL_RGB, GL_UNSIGNED_SHORT_5_6_5, &vec![0x55u8; (SIDE * SIDE * 2) as usize]);
    }

    #[test]
    fn an_rgba16f_texture_is_sampled() {
        let _g = rig();
        sampled(GL_RGBA16F, GL_RGBA, GL_HALF_FLOAT, &vec![0x11u8; (SIDE * SIDE * 8) as usize]);
    }

    #[test]
    fn an_r16f_texture_is_sampled() {
        let _g = rig();
        sampled(GL_R16F, GL_RED, GL_HALF_FLOAT, &vec![0x11u8; (SIDE * SIDE * 2) as usize]);
    }
}
