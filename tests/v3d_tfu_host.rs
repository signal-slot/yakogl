// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The host OSAL's model of the V3D's texture formatting unit
//! (`src/osal/linux.c`, "the TFU"): the bytes it writes are the bytes the
//! library's CPU path writes, so a host run with the TFU on samples the
//! same textures as a host run with it off.
//!
//! The model and the CPU path are both the real code, not copies of it:
//!
//! * The model: `src/osal/linux.c` built for the V3D with `driver/src/v3d/
//!   v3d_tex.c` (`cc`, as tests/v3d_rt_sample.rs builds its shim) and loaded
//!   with `dlopen`. A job is packed by the backend's packer
//!   (`v3d_tex_tfu_pack_mipmap`, `v3d_tex_tfu_pack`) and run through the
//!   OSAL's `query(OSAL_QUERY_V3D_TFU_RUN)`, as `src/v3d/backend.rs`
//!   `v3d_generate_mipmap` and `tfu_tile` submit it.
//! * The CPU path: the GL core's `glGenerateMipmap` (src/gl/texture.rs) on
//!   a stand-in back end of the V3D's shape that declines the chain
//!   (`generate_mipmap` absent, as the V3D answers without a TFU), its levels
//!   read from the texture's storage and tiled with `v3d_tex_store_level`
//!   into the same layout, as the backend's tiled copy of a mipmapped
//!   texture does (`copy_image_for`). A raster tiling job is compared with
//!   `v3d_tex_store_level` of the same rows (the CPU tiling it replaces).
//!
//! Sizes: power-of-two and not (glmark2's 800x600 terrain targets, odd
//! sides where the box drops the last row or column, one-texel-wide tails),
//! across the layouts Mesa picks (LT, UB-linear, UIF, UIF with level 0's
//! XOR dropped as `texture::rt_layout` drops it). Formats: RGBA8 (the type
//! the backend's mipmap job names), and LUMINANCE (R8) and
//! LUMINANCE_ALPHA (RG8), whose 8-bit channels the GL core averages the
//! same way; the raster jobs R8, R16F and R32F (the backend's blit types).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_tfu_host
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d"))]

use core::ffi::{c_char, c_int, c_void};
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use yakogl::backend::{gles_attribf_bit, gles_texfmt_bytes, GlesDraw, GlesHeap, YakoglBackend, GLES_ATTRIBF_FLOAT32};
use yakogl::gl::consts::*;
use yakogl::gl::state::glPixelStorei;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
}

/* ---- the model: linux.c and v3d_tex.c, built here ---------------------------- */

/// The shim over the real C. linux.c's ops table runs every job; the shim
/// lays the image out, fills level 0, packs the job with the backend's
/// packer, and copies the image out.
const SHIM: &str = r#"
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include "gles_osal.h"
#include "osal_internal.h"
#include "v3d_tex.h"

/* what linux.c takes from the rest of the library */
unsigned g_gl_error_log_limit;
bool v3d_job_pack_skip;

extern const struct osal_ops osal_linux_ops;
void osal_linux_gpu_time(uint64_t *wall_ns, uint64_t *cpu_ns, uint32_t *jobs);

/* init with the TFU on (`tfu` 1) or off; 0 or the init's error */
int shim_init(int tfu)
{
	osal_linux_ops.shutdown();
	setenv("YAKOGL_V3D_TFU", tfu ? "1" : "0", 1);
	return osal_linux_ops.init(NULL);
}

/* the answer to OSAL_QUERY_V3D_TFU_MIN_BYTES: the query's result, *out */
int shim_min_bytes(uint32_t *out)
{
	return osal_linux_ops.query(OSAL_QUERY_V3D_TFU_MIN_BYTES, out, sizeof(*out));
}

uint32_t shim_jobs(void)
{
	uint32_t jobs = 0;
	osal_linux_gpu_time(NULL, NULL, &jobs);
	return jobs;
}

/* The layout of a w x h image of `levels` levels; `rt` drops level 0's XOR
 * (src/v3d/texture.rs rt_layout). */
static bool layout(uint32_t w, uint32_t h, uint32_t levels, uint32_t cpp, int rt, struct v3d_tex_layout *l)
{
	if (!v3d_tex_layout(w, h, levels, cpp, l))
		return false;
	if (rt && l->slices[0].tiling == V3D_TEX_TILING_UIF_XOR)
		l->slices[0].tiling = V3D_TEX_TILING_UIF_NO_XOR;
	return true;
}

/* out[0] the image's size, out[1 + 3 * k ..] level k's offset, size and
 * tiling; 0 when refused */
int shim_layout(uint32_t w, uint32_t h, uint32_t levels, uint32_t cpp, int rt, uint32_t *out)
{
	struct v3d_tex_layout l;
	if (!layout(w, h, levels, cpp, rt, &l))
		return 0;
	out[0] = l.size;
	for (uint32_t k = 0; k < levels; k++) {
		out[1 + 3 * k] = l.slices[k].offset;
		out[2 + 3 * k] = l.slices[k].size;
		out[3 + 3 * k] = (uint32_t)l.slices[k].tiling;
	}
	return 1;
}

/* The CPU path's image: every level's raster rows (rows[k], strides[k]
 * bytes apart) tiled into a zeroed image (`out`, the layout's size). */
int shim_cpu_image(uint32_t w, uint32_t h, uint32_t levels, uint32_t cpp, int rt, const uint8_t *const *rows,
		   const uint32_t *strides, uint8_t *out)
{
	struct v3d_tex_layout l;
	if (!layout(w, h, levels, cpp, rt, &l))
		return 0;
	memset(out, 0, l.size);
	for (uint32_t k = 0; k < levels; k++)
		v3d_tex_store_level(out, &l, k, rows[k], strides[k]);
	return 1;
}

/* One TFU job through the OSAL on an image in GPU memory. `kind` 0: the
 * mipmap job of a render image whose level 0 is `level0` (rows of w * cpp
 * bytes); 1: the raster tiling of `level0` (in GPU memory too) into a
 * one-level image. `tweak` is XORed into the job word `tweak_word` (0..7:
 * icfg, iia, iis, ica, iua, ioa, ios, coef0) before it runs; `iia_off`
 * moves a raster source outside GPU memory when non-zero. The image is
 * copied to `out`. Returns the query's result; *rc the job's. */
int shim_job(int kind, uint32_t w, uint32_t h, uint32_t levels, uint32_t cpp, uint32_t ttype, const uint8_t *level0,
	     uint32_t tweak_word, uint32_t tweak, uint32_t iia_off, uint8_t *out, int32_t *rc)
{
	struct v3d_tex_layout l;
	struct osal_gpu_mem img = { 0 }, src = { 0 };
	struct v3d_tex_tfu_job j;
	int q = -1;

	*rc = 1;
	if (!layout(w, h, levels, cpp, kind == 0, &l))
		return -1000;
	if (osal_linux_ops.gpu_alloc(0, l.size, 0, &img) != 0)
		return -1001;
	if (kind == 0) {
		v3d_tex_store_level(img.cpu, &l, 0, level0, w * cpp);
		if (!v3d_tex_tfu_pack_mipmap(&l, ttype, (uint32_t)img.gpu, &j)) {
			q = -1002;
			goto out;
		}
	} else {
		if (osal_linux_ops.gpu_alloc(0, (size_t)w * h * cpp, 0, &src) != 0) {
			q = -1003;
			goto out;
		}
		memcpy(src.cpu, level0, (size_t)w * h * cpp);
		if (!v3d_tex_tfu_pack(&l, ttype, (uint32_t)src.gpu + iia_off, w * cpp, (uint32_t)img.gpu, &j)) {
			q = -1004;
			goto out;
		}
	}
	struct osal_v3d_tfu t = { j.icfg, j.iia, j.iis, j.ica, j.iua, j.ioa, j.ios, j.coef0, 0, 0 };
	uint32_t *words = &t.icfg;
	if (tweak_word < 8)
		words[tweak_word] ^= tweak;
	q = osal_linux_ops.query(OSAL_QUERY_V3D_TFU_RUN, &t, sizeof(t));
	*rc = t.rc;
	memcpy(out, img.cpu, l.size);
out:
	if (src.impl)
		osal_linux_ops.gpu_free(&src);
	osal_linux_ops.gpu_free(&img);
	return q;
}
"#;

fn lib() -> *mut c_void {
    static L: OnceLock<usize> = OnceLock::new();
    *L.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_tfu_host");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let src = dir.join(format!("shim-{}.c", std::process::id()));
        let so = dir.join(format!("shim-{}.so", std::process::id()));
        std::fs::write(&src, SHIM).expect("the shim source");
        /* linux.c with the warnings tools/check.sh's C rows use; the
         * compiler's temporary files next to the shim (v3d_rt_sample.rs) */
        let st = Command::new("cc")
            .current_dir(&root)
            .env("TMPDIR", &dir)
            .args(["-std=c11", "-D_GNU_SOURCE", "-O1", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
            .args(["-DYAKOGL_OSAL_LINUX", "-DYAKOGL_GPU_V3D", "-DGL_MAX_CONTEXTS=4"])
            .args(["-Iinclude", "-Isrc", "-Isrc/osal", "-Isrc/vivante", "-Idriver/src", "-Isrc/v3d"])
            .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated"])
            .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
            .arg("-Ithird_party/mesa-26.2.1/include")
            .arg(&src)
            .args(["src/osal/linux.c", "driver/src/v3d/v3d_tex.c", "driver/src/v3d/v3d_tiling_mesa.c"])
            .args(["-lm", "-o"])
            .arg(&so)
            .status()
            .expect("cc runs");
        assert!(st.success(), "the C shim over linux.c and v3d_tex.c builds");
        let path = CString::new(so.to_str().expect("a utf-8 path")).unwrap();
        // SAFETY: a path of a library this test just built; RTLD_NOW.
        let h = unsafe { dlopen(path.as_ptr(), 2) };
        assert!(!h.is_null(), "the shim loads");
        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_file(&so);
        h as usize
    }) as *mut c_void
}

fn sym(name: &str) -> *mut c_void {
    let c = CString::new(name).unwrap();
    // SAFETY: a handle `lib` made and a NUL-terminated name.
    let p = unsafe { dlsym(lib(), c.as_ptr()) };
    assert!(!p.is_null(), "{name} in the shim");
    p
}

/// The OSAL is one global state, and so is the GL core's current context:
/// the cases run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn model_init(tfu: bool) -> i32 {
    // SAFETY: shim_init's C signature.
    let f: extern "C" fn(c_int) -> c_int = unsafe { std::mem::transmute(sym("shim_init")) };
    f(c_int::from(tfu))
}

fn model_jobs() -> u32 {
    // SAFETY: shim_jobs' C signature.
    let f: extern "C" fn() -> u32 = unsafe { std::mem::transmute(sym("shim_jobs")) };
    f()
}

/// (image size, [(offset, size, tiling)] a level) of the layout.
fn layout(w: u32, h: u32, levels: u32, cpp: u32, rt: bool) -> (u32, Vec<(u32, u32, u32)>) {
    // SAFETY: shim_layout's C signature.
    let f: extern "C" fn(u32, u32, u32, u32, c_int, *mut u32) -> c_int =
        unsafe { std::mem::transmute(sym("shim_layout")) };
    let mut out = vec![0u32; 1 + 3 * levels as usize];
    assert_eq!(f(w, h, levels, cpp, c_int::from(rt), out.as_mut_ptr()), 1, "v3d_tex_layout refused {w}x{h}");
    (out[0], (0..levels as usize).map(|k| (out[1 + 3 * k], out[2 + 3 * k], out[3 + 3 * k])).collect())
}

/// A job's outcome: the query's result, the job's, and the image.
struct Job {
    query: i32,
    rc: i32,
    image: Vec<u8>,
}

/// The job word `tweak.0` XORed with `tweak.1` before it runs (8: none).
#[allow(clippy::too_many_arguments)]
fn model_job(
    kind: i32,
    w: u32,
    h: u32,
    levels: u32,
    cpp: u32,
    ttype: u32,
    level0: &[u8],
    tweak: (u32, u32),
    iia_off: u32,
) -> Job {
    type F = extern "C" fn(c_int, u32, u32, u32, u32, u32, *const u8, u32, u32, u32, *mut u8, *mut i32) -> c_int;
    // SAFETY: shim_job's C signature.
    let f: F = unsafe { std::mem::transmute(sym("shim_job")) };
    let (size, _) = layout(w, h, levels, cpp, kind == 0);
    let mut image = vec![0u8; size as usize];
    let mut rc = 0i32;
    let query =
        f(kind, w, h, levels, cpp, ttype, level0.as_ptr(), tweak.0, tweak.1, iia_off, image.as_mut_ptr(), &mut rc);
    Job { query, rc, image }
}

/// The CPU path's image of `levels`: each (rows, stride) tiled into the
/// layout.
fn cpu_image(w: u32, h: u32, cpp: u32, rt: bool, levels: &[(Vec<u8>, u32)]) -> Vec<u8> {
    type F = extern "C" fn(u32, u32, u32, u32, c_int, *const *const u8, *const u32, *mut u8) -> c_int;
    // SAFETY: shim_cpu_image's C signature.
    let f: F = unsafe { std::mem::transmute(sym("shim_cpu_image")) };
    let n = levels.len() as u32;
    let (size, _) = layout(w, h, n, cpp, rt);
    let rows: Vec<*const u8> = levels.iter().map(|(r, _)| r.as_ptr()).collect();
    let strides: Vec<u32> = levels.iter().map(|&(_, s)| s).collect();
    let mut out = vec![0u8; size as usize];
    assert_eq!(f(w, h, n, cpp, c_int::from(rt), rows.as_ptr(), strides.as_ptr(), out.as_mut_ptr()), 1);
    out
}

/* ---- the CPU path: the GL core's glGenerateMipmap ------------------------------ */

unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_h: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
    0
}

/// `yakogl_backend_v3d`'s flags the GL core reads (tests/es2_npot.rs
/// V3D_CAPS), with no `generate_mipmap`: the V3D's answer without a TFU,
/// so the GL core filters the chain on the CPU.
static V3D_SHAPE: YakoglBackend = YakoglBackend {
    name: c"tfu stand-in (V3D shape)".as_ptr() as *const c_char,
    native_attrib_formats: gles_attribf_bit(GLES_ATTRIBF_FLOAT32),
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: true,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: true,
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
    max_instance_divisor: 0xffff,
    wait_kick: None,
    samples_texel_copies: true,
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

unsafe extern "C" fn o_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice());
    let cpu = mem.as_mut_ptr() as *mut c_void;
    // SAFETY: `out` is the core's.
    unsafe { *out = OsalGpuMem { cpu, gpu: cpu as u64, phys: cpu as u64, size, impl_: cpu } };
    0
}
unsafe extern "C" fn o_gpu_free(_m: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn o_cache(_cpu: *mut c_void, _n: usize) {}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(o_gpu_alloc),
    gpu_free: Some(o_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(o_cache),
    cache_invd: Some(o_cache),
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

/// glTexImage2D of `level0` (`format`, UNSIGNED_BYTE) and glGenerateMipmap
/// on a fresh context: every level of the chain as (rows, stride in bytes),
/// read from the texture's storage where the GL core put it.
fn gl_core_chain(format: GLenum, w: u32, h: u32, level0: &[u8]) -> Vec<(Vec<u8>, u32)> {
    // SAFETY: the caller holds SERIAL; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &V3D_SHAPE;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(|| {
        let mut tex: GLuint = 0;
        // SAFETY: plain GL calls; `level0` holds the w x h texels.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                format as GLint,
                w as GLsizei,
                h as GLsizei,
                0,
                format,
                GL_UNSIGNED_BYTE,
                level0.as_ptr() as *const c_void,
            );
            glGenerateMipmap(GL_TEXTURE_2D);
            assert_eq!(yakogl::gl::state::glGetError(), GL_NO_ERROR, "glGenerateMipmap of {w}x{h}");
        }
        let c = yakogl::gl::context::current().expect("a current context");
        let t = c
            .objs_ref()
            .textures
            .iter()
            .find(|t| t.used && t.width == w && t.height == h && t.target == GL_TEXTURE_2D)
            .expect("the texture");
        let bpp = gles_texfmt_bytes(t.texfmt);
        let levels = u32::from(t.chain_levels);
        assert_eq!(levels, 32 - w.max(h).leading_zeros(), "{w}x{h}: GL's whole chain in the storage");
        // SAFETY: the texture's live storage, mem.size bytes.
        let mem = unsafe { core::slice::from_raw_parts(t.mem.cpu as *const u8, t.mem.size) };
        (0..levels)
            .map(|l| {
                let (lw, lh) = ((w >> l).max(1), (h >> l).max(1));
                let pitch = t.level_pitch_at(l) * bpp;
                let off = t.level_off[l as usize] as usize;
                (mem[off..off + ((lh - 1) * pitch + lw * bpp) as usize].to_vec(), pitch)
            })
            .collect::<Vec<_>>()
    });
    // SAFETY: the context made above.
    unsafe { yakogl::gl::context::gl_context_destroy(ctx) };
    r.unwrap_or_else(|e| std::panic::resume_unwind(e))
}

/* ---- the comparisons ------------------------------------------------------------- */

const R8: u32 = 0;
const RG8: u32 = 2;
const RGBA8: u32 = 4;
const R16F: u32 = 16;
const R32F: u32 = 29;

/// Texels no box filter can get right by accident: every byte its own.
fn texels(w: u32, h: u32, cpp: u32, seed: u32) -> Vec<u8> {
    let mut s = seed.wrapping_mul(2_654_435_761).wrapping_add(w * 31 + h);
    (0..w * h * cpp)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s >> 11) as u8
        })
        .collect()
}

/// The first differing level and byte of two images of one layout.
fn first_diff(a: &[u8], b: &[u8], slices: &[(u32, u32, u32)]) -> Option<(usize, usize)> {
    let i = a.iter().zip(b).position(|(x, y)| x != y)?;
    let level = slices.iter().position(|&(off, size, _)| (off..off + size).contains(&(i as u32)));
    Some((level.unwrap_or(usize::MAX), i))
}

/// The model's chain of `format`'s level 0 is the CPU path's, byte for byte
/// over the whole image (level 0 as stored, levels 1.. as filtered, the
/// padding as left), and the job is counted as GPU work.
fn mip_case(format: GLenum, ttype: u32, cpp: u32, w: u32, h: u32) {
    let level0 = texels(w, h, cpp, w ^ (h << 8) ^ ttype);
    /* the storage's rows are pair-padded (tex_pitch); the model is handed
     * the caller's tight rows, as the render image holds what the TLB
     * stored, and level 0's slice compares the two */
    let chain = gl_core_chain(format, w, h, &level0);
    let levels = chain.len() as u32;
    let cpu = cpu_image(w, h, cpp, true, &chain);
    let jobs = model_jobs();
    let job = model_job(0, w, h, levels, cpp, ttype, &level0, (8, 0), 0);
    assert_eq!((job.query, job.rc), (0, 0), "{w}x{h} type {ttype}: the job ran");
    assert_eq!(model_jobs(), jobs + 1, "the job counted as GPU work");
    let (_, slices) = layout(w, h, levels, cpp, true);
    assert_eq!(
        first_diff(&job.image, &cpu, &slices),
        None,
        "{w}x{h} type {ttype}: the model's image differs from the CPU path's (level, byte)"
    );
}

#[test]
fn rgba8_mip_chains_equal_the_gl_cores_filter() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(model_init(true), 0);
    /* POT (UIF levels, UB-linear and LT tails), refract's 1600x1200 at a
     * smaller scale, glmark2 terrain's 800x600 and its 256x256 bloom
     * targets, odd and one-texel-wide NPOT sides, a 1024x1024 whose level 0
     * Mesa lays out with the XOR the render image drops */
    for (w, h) in [(64, 64), (256, 256), (800, 600), (400, 300), (37, 21), (1, 9), (13, 1), (1024, 1024), (65, 130)] {
        mip_case(GL_RGBA, RGBA8, 4, w, h);
    }
}

#[test]
fn r8_and_rg8_mip_chains_equal_the_gl_cores_filter() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(model_init(true), 0);
    for (w, h) in [(128, 64), (100, 75), (7, 33)] {
        mip_case(GL_LUMINANCE, R8, 1, w, h);
        mip_case(GL_LUMINANCE_ALPHA, RG8, 2, w, h);
    }
}

/// A raster tiling job (`tfu_tile`'s) writes what the CPU tiling of the
/// same rows writes.
#[test]
fn raster_tiling_jobs_equal_the_cpu_tiling() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(model_init(true), 0);
    for (ttype, cpp) in [(R8, 1), (R16F, 2), (R32F, 4)] {
        for (w, h) in [(256, 256), (800, 600), (129, 67), (1, 300)] {
            let rows = texels(w, h, cpp, ttype ^ w);
            let job = model_job(1, w, h, 1, cpp, ttype, &rows, (8, 0), 0);
            assert_eq!((job.query, job.rc), (0, 0), "{w}x{h} type {ttype}: the job ran");
            let cpu = cpu_image(w, h, cpp, false, &[(rows, w * cpp)]);
            assert_eq!(job.image, cpu, "{w}x{h} type {ttype}: the model's tiling differs from the CPU's");
        }
    }
}

/// What the model refuses, in the job's result (the query answers 0, as
/// the board's does): a job word the backend's packer would not write
/// (-EINVAL), a float mipmap (-ENOTSUP: not modelled; the backend asks
/// none), a source outside GPU memory (-EFAULT, the board's MMU fault).
/// None of them writes the image. Without YAKOGL_V3D_TFU the queries are
/// not answered (-ENOTSUP), as on a board without the option, and the
/// backend tiles on the CPU.
#[test]
fn refusals_and_the_tfu_off() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(model_init(true), 0);
    let level0 = texels(64, 64, 4, 1);
    /* levels 1.. sit below level 0 in the image (Mesa's order): a refused
     * job leaves them as allocated, zero */
    let below_level0 = layout(64, 64, 7, 4, true).1[0].0 as usize;
    /* icfg's type a value no packer names (RGBA8 4 -> 5), iis of the UIF
     * level 0 off by one, ica (the packer writes 0) set, DIMTW cleared */
    for (word, bits) in [(0, 1 << 9), (2, 1), (3, 1), (5, 1)] {
        let job = model_job(0, 64, 64, 7, 4, RGBA8, &level0, (word, bits), 0);
        assert_eq!((job.query, job.rc), (0, -22), "word {word} ^ 0x{bits:x}");
        assert!(job.image[..below_level0].iter().all(|&b| b == 0), "word {word}: nothing written");
    }
    let job = model_job(0, 64, 64, 7, 4, R32F, &level0, (8, 0), 0);
    assert_eq!((job.query, job.rc), (0, -95), "a float mipmap is not modelled");
    assert!(job.image[..below_level0].iter().all(|&b| b == 0), "a float mipmap: nothing written");
    let job = model_job(1, 64, 64, 1, 4, R32F, &level0, (8, 0), 0x2000_0000);
    assert_eq!((job.query, job.rc), (0, -14), "a raster source past the GPU VA space");

    let mut min = 0u32;
    // SAFETY: shim_min_bytes' C signature.
    let f: extern "C" fn(*mut u32) -> c_int = unsafe { std::mem::transmute(sym("shim_min_bytes")) };
    assert_eq!((f(&mut min), min), (0, 65536), "the Kconfig default threshold");
    assert_eq!(model_init(false), 0);
    assert_eq!(f(&mut min), -95, "no TFU: the threshold query is not answered");
    let job = model_job(0, 64, 64, 7, 4, RGBA8, &level0, (8, 0), 0);
    assert_eq!(job.query, -95, "no TFU: the job query is not answered");
}
