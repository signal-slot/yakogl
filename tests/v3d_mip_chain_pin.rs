// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A safety net for the Pi 4 (V3D 4.2) mipmap chain: the contents of every
//! level of an RGBA8 chain at the sizes glmark2 terrain makes, as hashes
//! pinned here, so a later change to how the chain is built (TFU refusals and
//! fallbacks, copies, level scheduling, fewer kicks) shows that it leaves
//! every level's answer as it is.
//!
//! What the host build reaches, and what it does not:
//!
//! * The TFU path: `src/osal/linux.c`'s model of the V3D's TFU, run on the
//!   job `driver/src/v3d/v3d_tex.c` packs for `src/v3d/backend.rs`
//!   `v3d_generate_mipmap` (`v3d_tex_tfu_pack_mipmap`), both the real code
//!   (the shim is built with `cc` and loaded with `dlopen`, as in
//!   tests/v3d_tfu_host.rs). The hashes are of each level's slice of the
//!   tiled image the job writes (Mesa's layout, level 0 as the render image
//!   has it, its XOR dropped).
//! * The CPU fallback: the GL core's `glGenerateMipmap` (`src/gl/texture.rs`)
//!   on a back end of the V3D's shape that declines the chain
//!   (`generate_mipmap` absent, as the V3D answers without a TFU). The
//!   hashes are of each level's raster rows (`lw * 4` bytes a row, the
//!   storage's pair padding left out). The TFU image must equal these levels
//!   tiled: the CPU path is the reference filter the hardware path has to
//!   equal, and the test checks that too.
//! * Not reached: the real hardware, the kick order and the queueing behind
//!   a render job (tests/v3d_tfu_mipmap_nowait.rs pins the order), and
//!   `v3d_generate_mipmap` itself, which needs the job builder; the packed
//!   job and the model that runs it are the real ones.
//!
//! Sizes: terrain's FBO textures (docs/notes/v3d-terrain-copies.md): 800x600
//! (terrain, tilt-shift H), 256x256 (bloom H and V), 512x512 (the mipmapped
//! specular and diffuse textures), plus 1024x1024 and an NPOT case. Level 0
//! is a fixed xorshift pattern of the size (`texels`).
//!
//! The pinned values were generated from the code at d83e25b5 with
//! `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_mip_chain_pin
//! -- --ignored --nocapture`, which prints the table in this file's syntax. A change
//! that moves a hash changes the answer: justify it, then regenerate.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_mip_chain_pin
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_mip_chain_pin");
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

/* ---- the pins ---------------------------------------------------------------------- */

const RGBA8: u32 = 4;

/// glmark2 terrain: 800x600 (terrain, tilt-shift H), 256x256 (bloom H, V),
/// 512x512 (specular, diffuse, detail); 1024x1024 and an NPOT besides.
const SIZES: &[(u32, u32)] = &[(1024, 1024), (800, 600), (512, 512), (256, 256), (37, 21)];

/// Texels no box filter can get right by accident: every byte its own.
fn texels(w: u32, h: u32, seed: u32) -> Vec<u8> {
    let mut s = seed.wrapping_mul(2_654_435_761).wrapping_add(w * 31 + h);
    (0..w * h * 4)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s >> 11) as u8
        })
        .collect()
}

/// FNV-1a, 64 bits.
fn fnv(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes.into_iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// Per level: the hash of the GL core's raster rows (`lw * 4` bytes a row),
/// and of the TFU job's slice of the tiled image.
struct Pins {
    raster: Vec<u64>,
    tfu: Vec<u64>,
}

fn measure(w: u32, h: u32) -> Pins {
    let level0 = texels(w, h, w ^ (h << 8));
    let chain = gl_core_chain(GL_RGBA, w, h, &level0);
    let levels = chain.len() as u32;
    let raster = chain
        .iter()
        .enumerate()
        .map(|(l, (rows, pitch))| {
            let (lw, lh) = ((w >> l).max(1) as usize, (h >> l).max(1) as usize);
            let pitch = *pitch as usize;
            fnv((0..lh).flat_map(|y| rows[y * pitch..y * pitch + lw * 4].iter().copied()))
        })
        .collect();
    let jobs = model_jobs();
    let job = model_job(0, w, h, levels, 4, RGBA8, &level0, (8, 0), 0);
    assert_eq!((job.query, job.rc), (0, 0), "{w}x{h}: the TFU job ran");
    assert_eq!(model_jobs(), jobs + 1, "the job counted as GPU work");
    /* the reference filter the hardware path must equal, byte for byte over
     * the whole image (as tests/v3d_tfu_host.rs compares them) */
    let cpu = cpu_image(w, h, 4, true, &chain);
    assert_eq!(job.image, cpu, "{w}x{h}: the TFU image differs from the CPU path's");
    let (_, slices) = layout(w, h, levels, 4, true);
    let tfu = slices
        .iter()
        .map(|&(off, size, _)| fnv(job.image[off as usize..(off + size) as usize].iter().copied()))
        .collect();
    Pins { raster, tfu }
}

/// (w, h, raster hashes, TFU slice hashes), a level each.
type Pinned = (u32, u32, &'static [u64], &'static [u64]);

const PINNED: &[Pinned] = &[
    (
        1024,
        1024,
        &[
            0x3729adb18f563f22,
            0x9d8e2112518b9bcc,
            0xec1c9f775da77d6c,
            0x7f8a8a7936726783,
            0x1daaa43d5386fcb3,
            0x0482c68e78554e1a,
            0xbc3fbf96ac624cdd,
            0xf9c8e792fc44b4df,
            0x49df75e307261700,
            0x6008a1d3884bf2d6,
            0x601ce49ff4ced599,
        ],
        &[
            0xe0d00afea1f72482,
            0x5ec68ab59c84fe48,
            0x3fea2cb83c55f6c8,
            0x603c33f180086063,
            0xa49d1098d25dcfab,
            0x8897e4f87270e162,
            0x6cd1a836204909c1,
            0xf9e58844232bdfff,
            0x49df75e307261700,
            0xdab61f76eab5dcf6,
            0xd02902b676837589,
        ],
    ),
    (
        800,
        600,
        &[
            0x6a4a0d3ea3a2913e,
            0x781a3b2139c4ce35,
            0x947490efb884a34b,
            0x9277ae4958375508,
            0x304985a1d7340e15,
            0x76eb8ef3ba5d29c9,
            0x4124a1482abf375e,
            0xc3af51809fe552a3,
            0x4966cec66bd311f0,
            0x601ce49ff4ced599,
        ],
        &[
            0xae5b9af23ee470c2,
            0xe17b08f60dcd9379,
            0xc6078a89d0bf4417,
            0x79b0be672202423c,
            0xc58eb70145746ff9,
            0xd588cf9e83f9d1f5,
            0x1fd20f78e206a65e,
            0xe4a4e200f44e535b,
            0xfb2c946b0e000a50,
            0xd02902b676837589,
        ],
    ),
    (
        512,
        512,
        &[
            0x7711ae53d1229197,
            0x9144d692dc8ddad4,
            0x661c9be385111651,
            0xba388e52aa1fe2f9,
            0x4f3006d43522b21b,
            0xb24c2895d674e1a0,
            0x81c4399a321cbddd,
            0x51f88977e55b4678,
            0x3fbae917048205b6,
            0x984300ad293622fd,
        ],
        &[
            0x5872a040046c64a3,
            0x6ea5b48c572a8eb4,
            0x3a498032cf5abe31,
            0xc25d02c4b95e5ca9,
            0x4b1042c015accb8b,
            0x7ea755e2b8bc6aa4,
            0x10955ca56770bf91,
            0x51f88977e55b4678,
            0x42cf557d3c692db6,
            0x89e2a575389c52ad,
        ],
    ),
    (
        256,
        256,
        &[
            0xee8eea1bd9f6b265,
            0xea30c7c91a8521a1,
            0x685574a598da2b6e,
            0xb0070434e0993e85,
            0x0742df1bbe92f221,
            0x46d31033b0893089,
            0xa42a482d569bd6bd,
            0xc0702e6310a11099,
            0x5414349fed09728d,
        ],
        &[
            0x338e876e60805a9d,
            0x4fed51d6607dab59,
            0x1453077a68d8dc52,
            0x5df0d5016f31d649,
            0xbff76a1e34ae6dc1,
            0x93f87bb9bf05ce9d,
            0xa42a482d569bd6bd,
            0xda6c4fee0ab26179,
            0x779adfca9f43413d,
        ],
    ),
    (
        37,
        21,
        &[
            0xcae51bcf8dc83eee,
            0x4a975f93bb2e76a3,
            0x58cfd503bfd00f73,
            0xe52c9a18758f7600,
            0x3e06350075818b8b,
            0x323dea315a17d994,
        ],
        &[
            0xda0e2e3cf1c8f8ce,
            0xe6716babf29c6aa3,
            0xad19ab3f2014bc1b,
            0x6194f5a969c07600,
            0x95a141cf3159832b,
            0x2e3d1f36a084ae54,
        ],
    ),
];

/// Prints the table above from the current code (see the header).
#[test]
#[ignore = "prints the pinned table; run by hand to regenerate"]
fn print_the_pinned_table() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(model_init(true), 0);
    println!("const PINNED: &[Pinned] = &[");
    for &(w, h) in SIZES {
        let p = measure(w, h);
        println!("    ({w}, {h}, &{:#018x?}, &{:#018x?}),", p.raster, p.tfu);
    }
    println!("];");
}

#[test]
fn rgba8_mip_chain_levels_at_terrains_sizes_are_pinned() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(model_init(true), 0);
    assert_eq!(PINNED.len(), SIZES.len());
    for (&(w, h), &(pw, ph, raster, tfu)) in SIZES.iter().zip(PINNED) {
        assert_eq!((w, h), (pw, ph));
        let p = measure(w, h);
        assert_eq!(p.raster, raster, "{w}x{h}: the CPU path's levels moved");
        assert_eq!(p.tfu, tfu, "{w}x{h}: the TFU job's levels moved");
    }
}
