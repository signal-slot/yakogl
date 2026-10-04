// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A minimal EGL for platforms without a window system (`gl_egl.c`).
//!
//! One display (the OSAL), three configs (RGBA8888, the same with 4x
//! multisampling, RGB565; depth 24, stencil 8; window and pbuffer surfaces;
//! OpenGL ES 2/3), a small pool of contexts (the GL core's) and a few
//! surfaces. A window surface is the application's own pixel buffer
//! ([`GlesNativeWindow`], `struct gles_native_window` in EGL/eglplatform.h)
//! mapped for the GPU; a pbuffer surface is render-pool memory. Surfaces are
//! single-buffered: eglSwapBuffers finishes the frame. eglInitialize brings
//! up the OSAL and the PowerVR backend, so an application needs no other
//! entry point before GL.
//!
//! The entry points call each other in C (eglTerminate -> eglMakeCurrent,
//! ...). Here every entry point takes the EGL state once and the shared work
//! lives in helpers that get `&mut EglState` passed down, so no two
//! references to the state are ever live.
#![allow(non_snake_case)]

use core::ffi::{c_char, c_void};

use super::consts::{
    GL_INVALID_ENUM, GL_INVALID_OPERATION, GL_INVALID_VALUE, GL_RENDERBUFFER, GL_TEXTURE_2D, GL_TEXTURE_CUBE_MAP,
};
/// The largest native buffer side (the render-target limit).
use super::context::MAX_SURFACE_SIDE as MAX_NATIVE_SIDE;
use super::context::{self, gl_trace, GlContext, MAX_CONTEXTS};
use super::egl_consts::*;
use super::types::*;
use super::{draw, framebuffer, texture};
use crate::backend::{
    self, gles_surf_bytes_per_pixel, gles_texfmt_bytes, GLES_HEAP_DATA, GLES_SURF_RGB565, GLES_SURF_RGBA8888,
    GLES_SURF_XRGB8888, GLES_TEXFMT_RGB565, GLES_TEXFMT_RGBA8,
};
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA};
use crate::util::Global;

/* ---- platform types (EGL/eglplatform.h) ------------------------------------ */

/* Pixel byte order of a native window buffer. */
/// R,G,B,A in memory (as FBO textures).
pub const GLES_NATIVE_FORMAT_RGBA8888: u32 = 1;
/// B,G,R,X in memory (DRM XRGB8888 scan-out).
pub const GLES_NATIVE_FORMAT_XRGB8888: u32 = 2;
/// 16-bit R5G6B5, red in the high bits (DRM RGB565).
pub const GLES_NATIVE_FORMAT_RGB565: u32 = 3;
/// OR'd into a window's format: the display scans the buffer bottom row
/// first, so the surface is drawn y-up (`GlContext::default_y_up`). Only
/// windows take it; a native pixmap carrying it is refused.
pub const GLES_NATIVE_WINDOW_BOTTOM_UP: u32 = 0x8000_0000;

/// A native window (`struct gles_native_window`): `phys` is the buffer's
/// physical address, mapped for the GPU by eglCreateWindowSurface. The rows
/// are `stride_bytes` apart: the pair-padded `tex_pitch(width)` pixels, or,
/// for a window (not a pixmap), any wider pitch of whole pixels that is an
/// even number of them -- a view into a wider scan-out buffer
/// (`native_window_pitch`). The buffer stays owned by the application.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GlesNativeWindow {
    pub phys: u64,
    pub width: u32,
    pub height: u32,
    pub stride_bytes: u32,
    /// `GLES_NATIVE_FORMAT_*`.
    pub format: u32,
}

pub type EGLNativeWindowType = *mut GlesNativeWindow;
/// `GLES2/gl2ext.h` (GL_OES_EGL_image).
pub type GLeglImageOES = *mut c_void;
/// `__eglMustCastToProperFunctionPointerType`: `void (*)(void)`, NULL-able.
pub type EglProc = Option<unsafe extern "C" fn()>;

/// Surfaces that can exist at once. A slot is small -- the pixels are the
/// pbuffer's own allocation -- so this is a table size, not a memory budget,
/// and it rises for a build that needs it: `-DYAKOGL_MAX_SURFACES=<n>`
/// (CMake). dEQP's `multithread.pbuffer` holds up to six per thread.
const EGL_MAX_SURFACES: usize = context::env_usize(option_env!("YAKOGL_MAX_SURFACES"), 4);
const EGL_MAX_IMAGES: usize = 8;
const EGL_MAX_SYNCS: usize = 8;

const TRUE: EGLBoolean = EGL_TRUE as EGLBoolean;
const FALSE: EGLBoolean = EGL_FALSE as EGLBoolean;
const NO_SURFACE: EGLSurface = core::ptr::null_mut();
const NO_CONTEXT: EGLContext = core::ptr::null_mut();
const NO_DISPLAY: EGLDisplay = core::ptr::null_mut();
const NO_IMAGE: EGLImage = core::ptr::null_mut();
const NO_SYNC: EGLSync = core::ptr::null_mut();

/// An EGLImage: a w x h buffer (rows at the pair-padded pitch,
/// `GLES_SURF_*`). From a GL texture it takes over the texture's storage;
/// from a native buffer it maps the caller's pages.
///
/// Ownership: the slot whose `owner` is None holds the storage record. Every
/// GL texture / renderbuffer that uses the storage (the source texture and
/// each glEGLImageTarget* sibling, in any context) and every other image
/// made from such a texture (`owner` = this slot) counts one `refs`. The
/// storage is released when the image handle is destroyed (`used` false)
/// and `refs` is 0; until then the slot stays taken, so neither
/// eglDestroyImage nor a context destroy can free memory something still
/// samples or renders into.
#[derive(Clone, Copy)]
struct EglImage {
    /// The handle is valid (eglDestroyImage clears it).
    used: bool,
    /// Pool memory to free when the storage is released (a native buffer's
    /// mapping is the application's and stays).
    owned: bool,
    /// `GLES_SURF_*`.
    format: u8,
    width: u32,
    height: u32,
    mem: OsalGpuMem,
    /// The allocation `mem` is a part of, when it is one: a cube map's six
    /// faces are one allocation and an image of a face describes its own
    /// rows. `impl_` null when `mem` is the whole of it. The slot that holds
    /// the record frees this, not `mem`.
    alloc: OsalGpuMem,
    /// The slot holding the storage record when this image aliases another
    /// image's storage (made from a texture that is itself an image sibling).
    owner: Option<u8>,
    /// Users of the storage besides the handle (see above).
    refs: u32,
    /// The texels are in the pixel engine's B,G,R,A order with a real alpha
    /// (`GlTexture::pe_order`): made from a texture a draw rendered into, or
    /// from a colour renderbuffer on a backend whose RGBA8888 target is
    /// stored B,G,R,A. A texture the image is attached to takes it.
    pe_order: bool,
}

impl EglImage {
    const ZERO: EglImage = EglImage {
        used: false,
        owned: false,
        format: 0,
        width: 0,
        height: 0,
        mem: OsalGpuMem::ZERO,
        alloc: OsalGpuMem::ZERO,
        owner: None,
        refs: 0,
        pe_order: false,
    };

    /// The slot is taken: a live handle or storage still in use.
    fn live(&self) -> bool {
        self.used || self.refs != 0
    }

    /// The allocation the slot's storage record covers: `mem` unless the
    /// image is one part of a larger one.
    fn allocation(&self) -> &OsalGpuMem {
        if self.alloc.impl_.is_null() {
            &self.mem
        } else {
            &self.alloc
        }
    }
}

#[derive(Clone, Copy)]
struct EglSurface {
    used: bool,
    /// Else pbuffer.
    window: bool,
    /// A window the display scans bottom row first (GLES_NATIVE_WINDOW_BOTTOM_UP).
    bottom_up: bool,
    /// `GLES_SURF_*`.
    format: u8,
    /// From the config: 1 or 4.
    samples: u8,
    width: u32,
    height: u32,
    /// A window's row pitch in pixels when it is wider than
    /// `tex_pitch(width)` (a view into a wider scan-out buffer); 0 otherwise.
    pitch: u32,
    /// The colour buffer (window: mapped, not owned).
    mem: OsalGpuMem,
    /// EGL_CONFIG_ID of the config it was created with.
    config_id: u8,
    /// EGL_SWAP_BEHAVIOR set to EGL_BUFFER_DESTROYED. The surfaces preserve
    /// their colour buffer either way (they are single-buffered, rendered
    /// in place), so this only says the application has stopped relying on
    /// it -- but eglQuerySurface answers with what was set.
    swap_destroyed: bool,
}

impl EglSurface {
    const ZERO: EglSurface = EglSurface {
        used: false,
        window: false,
        bottom_up: false,
        format: 0,
        samples: 0,
        width: 0,
        height: 0,
        pitch: 0,
        mem: OsalGpuMem::ZERO,
        config_id: 0,
        swap_destroyed: false,
    };
}

/// An EGL context: its GL context is created on the first eglMakeCurrent (the
/// GL state starts from its defaults then) and lives until eglDestroyContext.
#[derive(Clone, Copy)]
struct EglContext {
    used: bool,
    gl: *mut GlContext,
    /// EGL_CONFIG_ID of the config it was created with.
    config_id: u8,
    /// The share group its objects live in: its own, or the one the
    /// `share_context` of eglCreateContext already had. The handle is held
    /// from eglCreateContext to eglDestroyContext, which is longer than the
    /// GL context lives -- that one is made by the first eglMakeCurrent.
    group: u8,
}

impl EglContext {
    const ZERO: EglContext =
        EglContext { used: false, gl: core::ptr::null_mut(), config_id: 0, group: context::GROUP_NONE };
}

/* EGL 1.5 fence syncs over the backend's kick sequence: a fence records
 * the id of the kick the frame's commands go into (flushing them first,
 * as EGL_SYNC_PRIOR_COMMANDS_COMPLETE requires), and is signaled once the
 * backend reports that kick complete. That lets a client that pipelines
 * (eglSwapBuffers only waits for the previous frame) find out when the
 * frame it just submitted has actually landed in its buffer, e.g. before
 * putting that buffer on a display, without eglWaitGL's full drain. */
#[derive(Clone, Copy)]
struct EglSyncObj {
    used: bool,
    /// The kick this fence waits for.
    kick: u32,
}

struct EglState {
    inited: bool,
    /// The native display: the OSAL init argument.
    platform_arg: *mut c_void,
    error: EGLint,
    api: EGLenum,
    ctx: [EglContext; MAX_CONTEXTS],
    surf: [EglSurface; EGL_MAX_SURFACES],
    img: [EglImage; EGL_MAX_IMAGES],
    syncs: [EglSyncObj; EGL_MAX_SYNCS],
    /// Slot in `ctx`; None: no context current.
    cur_ctx: Option<usize>,
    /// Slot in `surf`.
    cur_draw: Option<usize>,
    /// `eglSwapInterval`: vertical periods eglSwapBuffers waits out before it
    /// returns. EGL 1.5 3.10.3 makes 1 the default, and this is per display
    /// rather than per surface because YakoGL has one display.
    swap_interval: EGLint,
}

static E: Global<EglState> = Global::new(EglState {
    inited: false,
    platform_arg: core::ptr::null_mut(),
    /* EGL 1.5 3.1: eglGetError returns EGL_SUCCESS when nothing has gone
     * wrong, and EGL_SUCCESS is 0x3000, not 0. A zero-initialised `error`
     * made the very first eglGetError of a process report a code no client
     * recognises. Every entry point below clears it on entry, so the state
     * reflects the last call the way that section requires. */
    error: EGL_SUCCESS,
    api: 0,
    ctx: [EglContext::ZERO; MAX_CONTEXTS],
    surf: [EglSurface::ZERO; EGL_MAX_SURFACES],
    img: [EglImage::ZERO; EGL_MAX_IMAGES],
    syncs: [EglSyncObj { used: false, kick: 0 }; EGL_MAX_SYNCS],
    cur_ctx: None,
    cur_draw: None,
    swap_interval: 1,
});

/// The EGL state. Entry points take it once and pass it down.
///
/// One process-wide record. EGL 1.5 keeps the error status (3.1), the bound
/// client API (3.4) and the current context and surfaces (3.7.3) per thread;
/// here they are in this one record too, because one thread issues every GL
/// and EGL call of the process (the thread contract, `gles_osal.h`
/// "threads"). There is no per-thread state to keep apart. A call from a
/// second thread while a context is current is refused before it reaches
/// this record (`egl_entry!`).
#[inline]
fn state() -> &'static mut EglState {
    /* In the unit tests the "only thread in the library" is a promise the
     * test harness breaks: it runs tests on parallel threads. Tests
     * serialise through testutil's SERIAL (held by a live TestGl); one that
     * reaches the EGL state without it is a race, and fails here by name. */
    #[cfg(test)]
    assert!(
        crate::testutil::HOLDS_SERIAL.with(|c| c.get()),
        "EGL state reached without the test lock: hold a TestGl for the test (testutil::HOLDS_SERIAL)"
    );
    // SAFETY: the caller is the only thread in the library (the thread
    // contract; `egl_entry!` refuses a second one where the OSAL can tell
    // threads apart). Within a call the state is fetched once and passed to
    // helpers as a parameter, so no second reference is live.
    unsafe { E.get() }
}

/// What an EGL entry point returns when it refuses the call: its failure
/// value, which for every EGL return type is zero (EGL_FALSE, EGL_NO_DISPLAY,
/// EGL_NO_SURFACE, EGL_NO_CONTEXT, EGL_NO_IMAGE, EGL_NO_SYNC, a null string or
/// procedure, and 0 from eglClientWaitSync, which is not EGL_CONDITION_SATISFIED).
trait Refused {
    const REFUSED: Self;
}
impl Refused for EGLBoolean {
    const REFUSED: Self = FALSE;
}
impl Refused for EGLint {
    const REFUSED: Self = 0;
}
impl Refused for *mut c_void {
    const REFUSED: Self = core::ptr::null_mut();
}
impl Refused for *const c_char {
    const REFUSED: Self = core::ptr::null();
}
impl Refused for EglProc {
    const REFUSED: Self = None;
}

fn refused<T: Refused>() -> T {
    T::REFUSED
}

/// Opens every EGL entry point. A call from a thread other than the one
/// with a context current is refused first (`context::foreign_thread`,
/// which logs it once): the entry returns its failure value and touches no
/// state, and that thread's eglGetError answers EGL_BAD_ACCESS. Then it
/// takes the OSAL's entry-point lock, held to the end of the entry.
macro_rules! egl_entry {
    () => {
        if context::foreign_thread() {
            return refused();
        }
        let _lock = osal::Lock::take();
    };
}

/// THE_DISPLAY: the address of the state.
#[inline]
fn the_display() -> EGLDisplay {
    E.as_ptr() as EGLDisplay
}

/* The configs: 32-bit RGBA (also XRGB8888 windows), the same with 4x
 * multisampling, and 16-bit RGB565 (RGB565 native windows). */
const CONFIG_1X: EGLConfig = 1 as EGLConfig;
const CONFIG_4X: EGLConfig = 2 as EGLConfig;
const CONFIG_565: EGLConfig = 3 as EGLConfig;
/// The configs, in handle order. The GC7000 build offers the 4x one only
/// while its backend multisamples (see [`offered_configs`]): a 4x surface rendered
/// single-sampled while the config said EGL_SAMPLES 4 -- gl_test 16's edges,
/// and every application that asks for 4x, told something the draw does not
/// do (8d0f4b9).
const ALL_CONFIGS: [EGLConfig; 3] = [CONFIG_1X, CONFIG_4X, CONFIG_565];
/// The largest list; what a caller's array is sized against.
const NUM_CONFIGS: EGLint = ALL_CONFIGS.len() as EGLint;

/// The configs this build offers now. On the GC7000 the 4x config is listed
/// only while its backend multisamples (`vivante::backend::msaa_on`: off
/// until the board has measured the resolve, `gc_debug_set_msaa` turns it
/// on for that measurement).
fn offered_configs() -> &'static [EGLConfig] {
    /* The V3D build offers the 4x config only while its backend's MSAA
     * switch is on (`v3d::backend::msaa_on`, off until the board has measured
     * it): a config that says EGL_SAMPLES 4 over a single-sampled draw is the
     * defect 8d0f4b9 removed on the GC7000. */
    #[cfg(feature = "gpu-v3d")]
    {
        const WITHOUT_4X: [EGLConfig; 2] = [CONFIG_1X, CONFIG_565];
        if !crate::v3d::backend::msaa_on() {
            return &WITHOUT_4X;
        }
    }
    #[cfg(feature = "gpu-vivante")]
    {
        const WITHOUT_4X: [EGLConfig; 2] = [CONFIG_1X, CONFIG_565];
        if !crate::vivante::backend::msaa_on() {
            return &WITHOUT_4X;
        }
    }
    &ALL_CONFIGS
}

fn valid_config(c: EGLConfig) -> bool {
    offered_configs().contains(&c)
}

/// EGL_CONFIG_ID of a valid config (the handles are the ids).
fn config_id(c: EGLConfig) -> u8 {
    c as usize as u8
}

fn config_samples(c: EGLConfig) -> u8 {
    if c == CONFIG_4X {
        4
    } else {
        1
    }
}

/// The colour format a config's surfaces have. EGL_RED_SIZE 5 is the 565
/// config; the other two are RGBA8888.
///
/// A pbuffer used to take this from nowhere and stay at the zero of
/// `EglSurface`, which is RGBA8888: a pbuffer on the 565 config was
/// allocated, rendered and read back as RGBA8888. dEQP-EGL saw the colour
/// channels off by exactly 565's quantisation and the alpha channel
/// carrying whatever had been cleared into it, where a format with no alpha
/// bits must read back opaque.
fn config_surf_format(c: EGLConfig) -> u8 {
    if c == CONFIG_565 {
        GLES_SURF_RGB565
    } else {
        GLES_SURF_RGBA8888
    }
}

/// `GLES_NATIVE_FORMAT_*` of a native window/buffer -> `GLES_SURF_*` (None: unknown).
fn native_format_to_surf(f: u32) -> Option<u8> {
    match f {
        GLES_NATIVE_FORMAT_RGBA8888 => Some(GLES_SURF_RGBA8888),
        GLES_NATIVE_FORMAT_XRGB8888 => Some(GLES_SURF_XRGB8888),
        GLES_NATIVE_FORMAT_RGB565 => Some(GLES_SURF_RGB565),
        _ => None,
    }
}

/// The surface format of a well-formed native buffer description: non-empty,
/// backed by memory, a known format and rows at the pair-padded pitch the
/// PBE writes and the TPU samples (`tex_pitch(width)` pixels: tightly packed
/// for an even width; an odd-width buffer must carry the pad pixel, else
/// the GPU would skew the rows and write past the buffer's last row).
fn native_buffer_format(nb: &GlesNativeWindow) -> Option<u8> {
    let format = native_window_format(nb)?;
    (nb.stride_bytes == context::tex_pitch(nb.width) * gles_surf_bytes_per_pixel(format)).then_some(format)
}

/// The surface format and row pitch in pixels of a native WINDOW: as
/// [`native_buffer_format`], except that the rows may be further apart than
/// `tex_pitch(width)`, a view into a wider scan-out buffer (a window smaller
/// than the panel, shown in place). The pitch must be:
/// - whole pixels (stride_bytes a multiple of the pixel size);
/// - an even number of them, at least `tex_pitch(width)`: every GPU writes
///   whole pixel pairs (gl/context.rs `tex_pitch`; the PowerVR's PBE line
///   stride counts pixel pairs, pvr/backend.rs `pbe_reg_words`);
/// - at most MAX_NATIVE_SIDE pixels, which every stride field holds: the
///   PowerVR's linestride is 16 bits of pairs (pbestate.h, bits 2..17 of
///   word 1), the V3D's raster store stride 20 bits of bytes
///   (v3d_packet.xml, "Height in UB or Stride", size 20), the GC7000's
///   PE_COLOR_STRIDE a 32-bit byte count; the PowerVR load quad's texture
///   stride is 15 bits of texels (texstate.h, STRIDE_IMAGE_WORD1 bits 0..14).
///
/// The display's own alignment (the i.MX8MP LCDIF's 64-byte PITCH,
/// ip/lcdifv3.rs) is the board layer's to meet when it hands out the view.
fn native_window_pitch(nb: &GlesNativeWindow) -> Option<(u8, u32)> {
    let format = native_window_format(nb)?;
    let bpp = gles_surf_bytes_per_pixel(format);
    if !nb.stride_bytes.is_multiple_of(bpp) {
        return None;
    }
    let pitch = nb.stride_bytes / bpp;
    (pitch.is_multiple_of(2) && pitch >= context::tex_pitch(nb.width) && pitch <= MAX_NATIVE_SIDE)
        .then_some((format, pitch))
}

/// A native buffer's format, before its pitch is judged: backed, non-empty,
/// within MAX_NATIVE_SIDE, a known format.
fn native_window_format(nb: &GlesNativeWindow) -> Option<u8> {
    if nb.phys == 0 || nb.width == 0 || nb.height == 0 || nb.width > MAX_NATIVE_SIDE || nb.height > MAX_NATIVE_SIDE {
        return None;
    }
    native_format_to_surf(nb.format)
}

/// Bytes of a validated native buffer (`native_buffer_format`): rows of
/// `stride_bytes`, at most 16384 * 16384 * 4 (fits a u64/usize product).
fn native_buffer_bytes(nb: &GlesNativeWindow) -> usize {
    nb.height as usize * nb.stride_bytes as usize
}

impl EglState {
    fn fail(&mut self, err: EGLint) -> EGLBoolean {
        self.error = err;
        FALSE
    }

    fn valid_display(&mut self, dpy: EGLDisplay) -> bool {
        if dpy != the_display() {
            self.error = EGL_BAD_DISPLAY;
            return false;
        }
        if !self.inited {
            self.error = EGL_NOT_INITIALIZED;
            return false;
        }
        true
    }

    /* Handles are the addresses of the pool slots. */

    fn context_handle(&mut self, i: usize) -> EGLContext {
        &raw mut self.ctx[i] as EGLContext
    }

    fn surface_handle(&mut self, i: usize) -> EGLSurface {
        &raw mut self.surf[i] as EGLSurface
    }

    fn image_handle(&mut self, i: usize) -> EGLImage {
        &raw mut self.img[i] as EGLImage
    }

    fn sync_handle(&mut self, i: usize) -> EGLSync {
        &raw mut self.syncs[i] as EGLSync
    }

    fn lookup_context(&mut self, c: EGLContext) -> Option<usize> {
        (0..MAX_CONTEXTS).find(|&i| self.ctx[i].used && self.context_handle(i) == c)
    }

    fn lookup_surface(&mut self, s: EGLSurface) -> Option<usize> {
        (0..EGL_MAX_SURFACES).find(|&i| self.surf[i].used && self.surface_handle(i) == s)
    }

    /// The storage record of `mem` (see [`EglImage`]).
    ///
    /// `mem` need not be the whole of what the slot holds: an image of one
    /// cube-map face describes its own rows of an allocation the record
    /// covers entirely, and the texture, the other faces' images and their
    /// siblings all have to find that one record.
    fn storage_slot(&self, mem: &OsalGpuMem) -> Option<usize> {
        (0..EGL_MAX_IMAGES).find(|&i| {
            let s = &self.img[i];
            if !s.live() || s.owner.is_some() || mem.impl_.is_null() {
                return false;
            }
            let a = s.allocation();
            a.impl_ == mem.impl_ && mem.gpu >= a.gpu && mem.gpu - a.gpu <= (a.size - mem.size.min(a.size)) as u64
        })
    }

    /// One user less for storage slot `i`; release the storage when the
    /// handle is gone and nothing uses it any more.
    fn storage_unref(&mut self, i: usize) {
        let s = &mut self.img[i];
        s.refs = s.refs.saturating_sub(1);
        if !s.live() {
            self.storage_release(i);
        }
    }

    /// The last user of slot `i` is gone: free what the slot holds.
    fn storage_release(&mut self, i: usize) {
        let s = &mut self.img[i];
        let owner = s.owner;
        if owner.is_none() && s.owned && !s.allocation().impl_.is_null() {
            /* the whole allocation goes, not the part `mem` describes */
            let mut m = *s.allocation();
            /* the OSAL defers the free until the kicks recorded so far are done */
            osal::gpu_free(&mut m);
        }
        self.img[i] = EglImage::ZERO;
        if let Some(o) = owner {
            self.storage_unref(o as usize);
        }
    }

    fn lookup_image(&mut self, img: EGLImage) -> Option<usize> {
        (0..EGL_MAX_IMAGES).find(|&i| self.img[i].used && self.image_handle(i) == img)
    }

    fn lookup_sync(&mut self, s: EGLSync) -> Option<usize> {
        (0..EGL_MAX_SYNCS).find(|&i| self.syncs[i].used && self.sync_handle(i) == s)
    }

    fn alloc_surface_slot(&mut self) -> Option<usize> {
        let i = self.surf.iter().position(|s| !s.used)?;
        self.surf[i] = EglSurface::ZERO;
        Some(i)
    }
}

fn sync_signaled(sync: &EglSyncObj) -> bool {
    !backend::kick_pending(sync.kick)
}

/// gl_flush_pending_clear(): emit the clear-only kick of a glClear no draw followed.
fn flush_pending_clear() {
    if let Some(c) = context::current() {
        draw::flush_pending_clear(c);
    }
}

/// # Safety
/// `list` must be null or an EGL_NONE-terminated attribute list.
unsafe fn attrib_of(list: *const EGLint, key: EGLint, def: EGLint) -> EGLint {
    if list.is_null() {
        return def;
    }
    let mut found = def;
    let mut a = list;
    /* the application's array: read without assuming alignment */
    while a.read_unaligned() != EGL_NONE {
        if a.read_unaligned() == key {
            /* the list is a map: a name given twice takes its later value.
             * dEQP's choose_config.random builds lists with repeats and
             * expects the last one to decide. */
            found = a.add(1).read_unaligned();
        }
        a = a.add(2);
    }
    found
}

/* ---- display ------------------------------------------------------------- */

#[no_mangle]
pub unsafe extern "C" fn eglGetDisplay(display_id: EGLNativeDisplayType) -> EGLDisplay {
    egl_entry!();
    get_display(display_id)
}

/// eglGetDisplay, under the lock its callers hold (eglGetPlatformDisplay is
/// the same call with a platform argument this EGL has one of).
fn get_display(display_id: EGLNativeDisplayType) -> EGLDisplay {
    gl_trace!(eglGetDisplay);
    let e = state();
    e.error = EGL_SUCCESS;
    if !e.inited {
        e.platform_arg = display_id;
    }
    the_display()
}

#[no_mangle]
pub unsafe extern "C" fn eglGetPlatformDisplay(
    _platform: EGLenum,
    native_display: *mut c_void,
    _attrib_list: *const EGLAttrib,
) -> EGLDisplay {
    egl_entry!();
    gl_trace!(eglGetPlatformDisplay);
    state().error = EGL_SUCCESS;
    get_display(native_display)
}

#[no_mangle]
pub unsafe extern "C" fn eglInitialize(dpy: EGLDisplay, major: *mut EGLint, minor: *mut EGLint) -> EGLBoolean {
    egl_entry!();
    gl_trace!(eglInitialize);
    let e = state();
    e.error = EGL_SUCCESS;
    if dpy != the_display() {
        return e.fail(EGL_BAD_DISPLAY);
    }
    if !e.inited {
        let Some(init) = osal::ops().and_then(|o| o.init) else {
            return e.fail(EGL_NOT_INITIALIZED);
        };
        /* the OSAL init is repeatable on the platforms this library
         * targets (a harness may have brought the GPU up already) */
        // SAFETY: the OSAL's init takes the native display argument as is.
        if unsafe { init(e.platform_arg) } != 0 || backend::init() != 0 {
            return e.fail(EGL_NOT_INITIALIZED);
        }
        e.inited = true;
        e.api = EGL_OPENGL_ES_API as EGLenum;
    }
    // SAFETY: major/minor are null or valid for a write (EGL contract).
    unsafe {
        if !major.is_null() {
            major.write_unaligned(1);
        }
        if !minor.is_null() {
            minor.write_unaligned(5);
        }
    }
    e.error = EGL_SUCCESS;
    TRUE
}

#[no_mangle]
pub unsafe extern "C" fn eglTerminate(dpy: EGLDisplay) -> EGLBoolean {
    egl_entry!();
    gl_trace!(eglTerminate);
    let e = state();
    e.error = EGL_SUCCESS;
    if dpy != the_display() {
        return e.fail(EGL_BAD_DISPLAY);
    }
    if e.inited {
        make_current(e, dpy, NO_SURFACE, NO_SURFACE, NO_CONTEXT);
        for i in 0..EGL_MAX_SURFACES {
            if e.surf[i].used {
                let h = e.surface_handle(i);
                destroy_surface(e, dpy, h);
            }
        }
        for i in 0..MAX_CONTEXTS {
            if e.ctx[i].used {
                let h = e.context_handle(i);
                destroy_context(e, dpy, h);
            }
        }
        for i in 0..EGL_MAX_IMAGES {
            if e.img[i].used {
                let h = e.image_handle(i);
                destroy_image(e, dpy, h);
            }
        }
        /* EGL 1.5 3.2: every resource of the display goes, sync objects too
         * (they hold nothing but their pool slot) */
        for s in e.syncs.iter_mut() {
            s.used = false;
        }
        /* the GPU stays up: the OSAL/backend serve the next eglInitialize */
        e.inited = false;
    }
    e.error = EGL_SUCCESS;
    TRUE
}

#[no_mangle]
pub unsafe extern "C" fn eglQueryString(dpy: EGLDisplay, name: EGLint) -> *const c_char {
    egl_entry!();
    gl_trace!(eglQueryString);
    let e = state();
    e.error = EGL_SUCCESS;
    if name == EGL_EXTENSIONS && dpy == NO_DISPLAY {
        /* EGL_EXT_client_extensions is this query itself: an extension
         * string without a display. Naming it is how an application tells
         * the query apart from the EGL_BAD_DISPLAY of an EGL that does not
         * have it -- an empty string says the opposite, that the mechanism
         * works and the extension it is named after is missing. */
        return c"EGL_EXT_client_extensions".as_ptr();
    }
    if !e.valid_display(dpy) {
        return core::ptr::null();
    }
    match name {
        EGL_VENDOR => c"YakoGL (PowerVR BXS-4-64)".as_ptr(),
        EGL_VERSION => c"1.5 YakoGL".as_ptr(),
        /* EGL_KHR_create_context: EGL 1.5 made it core, and eglCreateContext
         * takes its attributes (the major / minor version, the flags); dEQP's
         * ES 3 context creation asks for the extension by name all the same
         * (egluGLUtil.cpp) and refuses to run a GLES3 case without it. */
        EGL_EXTENSIONS => {
            c"EGL_KHR_image_base EGL_KHR_gl_texture_2D_image EGL_KHR_image_pixmap EGL_KHR_create_context".as_ptr()
        }
        EGL_CLIENT_APIS => c"OpenGL_ES".as_ptr(),
        _ => {
            e.error = EGL_BAD_PARAMETER;
            core::ptr::null()
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn eglGetError() -> EGLint {
    if context::foreign_thread() {
        /* a second thread: its calls were refused, and the owner's error
         * is not its to take (see egl_entry!) */
        return EGL_BAD_ACCESS;
    }
    let _lock = osal::Lock::take();
    gl_trace!(eglGetError);
    let e = state();
    let err = e.error;

    e.error = EGL_SUCCESS;
    err
}

#[no_mangle]
pub unsafe extern "C" fn eglBindAPI(api: EGLenum) -> EGLBoolean {
    egl_entry!();
    gl_trace!(eglBindAPI);
    let e = state();
    e.error = EGL_SUCCESS;
    if api != EGL_OPENGL_ES_API as EGLenum {
        return e.fail(EGL_BAD_PARAMETER);
    }
    e.api = api;
    TRUE
}

#[no_mangle]
pub unsafe extern "C" fn eglQueryAPI() -> EGLenum {
    egl_entry!();
    gl_trace!(eglQueryAPI);
    let e = state();
    e.error = EGL_SUCCESS;
    if e.api != 0 {
        e.api
    } else {
        EGL_OPENGL_ES_API as EGLenum
    }
}

#[no_mangle]
pub unsafe extern "C" fn eglReleaseThread() -> EGLBoolean {
    egl_entry!();
    gl_trace!(eglReleaseThread);
    state().error = EGL_SUCCESS;
    TRUE
}

/* ---- configs ------------------------------------------------------------- */

/// eglGetConfigAttrib for a valid config: the value, or None for an unknown attribute.
fn config_attrib(config: EGLConfig, attribute: EGLint) -> Option<EGLint> {
    let is565 = config == CONFIG_565;
    let is4x = config == CONFIG_4X;
    Some(match attribute {
        EGL_BUFFER_SIZE => {
            if is565 {
                16
            } else {
                32
            }
        }
        EGL_RED_SIZE | EGL_BLUE_SIZE => {
            if is565 {
                5
            } else {
                8
            }
        }
        EGL_GREEN_SIZE => {
            if is565 {
                6
            } else {
                8
            }
        }
        EGL_ALPHA_SIZE => {
            if is565 {
                0
            } else {
                8
            }
        }
        EGL_LUMINANCE_SIZE | EGL_ALPHA_MASK_SIZE => 0,
        EGL_DEPTH_SIZE => 24,
        EGL_STENCIL_SIZE => 8,
        EGL_SAMPLES => {
            if is4x {
                4
            } else {
                0
            }
        }
        EGL_SAMPLE_BUFFERS => {
            if is4x {
                1
            } else {
                0
            }
        }
        /* the colour buffer survives a swap: one buffer, rendered in place */
        EGL_SURFACE_TYPE => EGL_WINDOW_BIT | EGL_PBUFFER_BIT | EGL_SWAP_BEHAVIOR_PRESERVED_BIT,
        /* ES 3 contexts can be created (Qt's RHI asks for one), so the
         * renderable type names both. Conformance is a different claim: EGL
         * 1.5 3.4 says EGL_CONFORMANT tells "if a client API context created
         * with respect to the corresponding EGLConfig will pass the required
         * conformance tests for that API". An ES 3 context would not:
         * glBlitFramebuffer always fails (src/GL_CORE_NOTES.md, "What is not
         * ES 3.0 conformant"). So only the ES 2 bit is claimed. */
        EGL_RENDERABLE_TYPE => EGL_OPENGL_ES2_BIT | EGL_OPENGL_ES3_BIT,
        EGL_CONFORMANT => EGL_OPENGL_ES2_BIT,
        EGL_CONFIG_ID => {
            if is565 {
                3
            } else if is4x {
                2
            } else {
                1
            }
        }
        EGL_NATIVE_RENDERABLE => EGL_TRUE,
        EGL_NATIVE_VISUAL_ID => 0,
        EGL_NATIVE_VISUAL_TYPE => EGL_NONE,
        EGL_MAX_PBUFFER_WIDTH | EGL_MAX_PBUFFER_HEIGHT => 4096,
        EGL_MAX_PBUFFER_PIXELS => 4096 * 4096,
        EGL_MIN_SWAP_INTERVAL => 0,
        EGL_MAX_SWAP_INTERVAL => 1,
        EGL_COLOR_BUFFER_TYPE => EGL_RGB_BUFFER,
        EGL_LEVEL => 0,
        EGL_TRANSPARENT_TYPE => EGL_NONE,
        EGL_TRANSPARENT_RED_VALUE | EGL_TRANSPARENT_GREEN_VALUE | EGL_TRANSPARENT_BLUE_VALUE => 0,
        EGL_BIND_TO_TEXTURE_RGB | EGL_BIND_TO_TEXTURE_RGBA => EGL_FALSE,
        EGL_CONFIG_CAVEAT => EGL_NONE,
        _ => return None,
    })
}

#[no_mangle]
pub unsafe extern "C" fn eglGetConfigAttrib(
    dpy: EGLDisplay,
    config: EGLConfig,
    attribute: EGLint,
    value: *mut EGLint,
) -> EGLBoolean {
    egl_entry!();
    gl_trace!(eglGetConfigAttrib);
    let e = state();
    e.error = EGL_SUCCESS;
    if !e.valid_display(dpy) {
        return FALSE;
    }
    if !valid_config(config) {
        return e.fail(EGL_BAD_CONFIG);
    }
    if value.is_null() {
        return e.fail(EGL_BAD_PARAMETER);
    }
    match config_attrib(config, attribute) {
        Some(v) => {
            // SAFETY: value is non-null and valid for a write (EGL
            // contract), not necessarily aligned.
            unsafe { value.write_unaligned(v) };
            TRUE
        }
        None => e.fail(EGL_BAD_ATTRIBUTE),
    }
}

#[no_mangle]
pub unsafe extern "C" fn eglGetConfigs(
    dpy: EGLDisplay,
    configs: *mut EGLConfig,
    config_size: EGLint,
    num_config: *mut EGLint,
) -> EGLBoolean {
    egl_entry!();
    gl_trace!(eglGetConfigs);
    let e = state();
    e.error = EGL_SUCCESS;
    if !e.valid_display(dpy) {
        return FALSE;
    }
    if num_config.is_null() {
        return e.fail(EGL_BAD_PARAMETER);
    }
    // SAFETY: configs is null or valid for config_size entries, num_config
    // is non-null and valid for a write (EGL contract).
    unsafe {
        if !configs.is_null() {
            for (i, cfg) in offered_configs().iter().enumerate().take(config_size.max(0) as usize) {
                configs.add(i).write_unaligned(*cfg);
            }
        }
        {
            let n = offered_configs().len() as EGLint;
            num_config.write_unaligned(if configs.is_null() { n } else { config_size.clamp(0, n) });
        }
    }
    TRUE
}

/// A config matches when every requested attribute is satisfiable: sizes at
/// most its own, bitmask attributes a subset of its own. EGL's "AtLeast"
/// attributes (colour sizes, samples) sort smaller-first, so the
/// single-sampled config comes before the multisampled one unless samples
/// were asked for. `Err(())`: the list names an unknown attribute.
///
/// # Safety
/// `attrib_list` must be null or an EGL_NONE-terminated attribute list.
unsafe fn config_matches(cfg: EGLConfig, attrib_list: *const EGLint) -> Result<bool, ()> {
    if attrib_list.is_null() {
        return Ok(true);
    }
    /* First the names, so that the list's length is known before any value
     * is read: dEQP passes the one-element list {0xffffffff}, with nothing
     * behind it. (That name is also EGL_DONT_CARE's bit pattern, so a walk
     * that steps over it reads past the end.) */
    let mut pairs = 0usize;
    let mut a = attrib_list;
    /* the application's array: read without assuming alignment */
    while a.read_unaligned() != EGL_NONE {
        if !config_criterion(a.read_unaligned()) {
            return Err(());
        }
        pairs += 1;
        a = a.add(2);
    }
    for i in 0..pairs {
        let key = attrib_list.add(2 * i).read_unaligned();
        /* the list is a map: a name given twice takes its later value */
        if (i + 1..pairs).any(|j| attrib_list.add(2 * j).read_unaligned() == key) {
            continue;
        }
        let want = attrib_list.add(2 * i + 1).read_unaligned();
        if want == EGL_DONT_CARE {
            continue;
        }
        if !config_value_ok(key, want) {
            return Err(()); /* a value outside the attribute's set */
        }
        /* EGL_MATCH_NATIVE_PIXMAP names a pixmap, not a value to compare */
        let Some(have) = config_attrib(cfg, key) else {
            continue;
        };
        gl_trace!(eglGetConfigAttrib); /* the C asks the entry point */
        let matches = match key {
            /* mask: every bit asked for is one the config has */
            EGL_SURFACE_TYPE | EGL_RENDERABLE_TYPE | EGL_CONFORMANT => (want & !have) == 0,
            /* at least: the config's own size may exceed the request */
            EGL_BUFFER_SIZE | EGL_RED_SIZE | EGL_GREEN_SIZE | EGL_BLUE_SIZE | EGL_ALPHA_SIZE | EGL_DEPTH_SIZE
            | EGL_STENCIL_SIZE | EGL_SAMPLES | EGL_SAMPLE_BUFFERS | EGL_LUMINANCE_SIZE | EGL_ALPHA_MASK_SIZE => {
                want <= have
            }
            /* the transparent values are read only for a transparent config,
             * and no config here is one */
            EGL_TRANSPARENT_RED_VALUE | EGL_TRANSPARENT_GREEN_VALUE | EGL_TRANSPARENT_BLUE_VALUE => true,
            EGL_NATIVE_VISUAL_ID => true, /* EGL 1.5 3.4.1: ignored */
            /* exact: EGL 1.5 table 3.4's remaining criteria */
            _ => want == have,
        };
        if !matches {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Whether EGL 1.5 table 3.4 lists `key` as a config selection criterion,
/// which is what eglChooseConfig accepts in its attribute list.
fn config_criterion(key: EGLint) -> bool {
    matches!(
        key,
        EGL_BUFFER_SIZE
            | EGL_RED_SIZE
            | EGL_GREEN_SIZE
            | EGL_BLUE_SIZE
            | EGL_ALPHA_SIZE
            | EGL_LUMINANCE_SIZE
            | EGL_ALPHA_MASK_SIZE
            | EGL_DEPTH_SIZE
            | EGL_STENCIL_SIZE
            | EGL_SAMPLES
            | EGL_SAMPLE_BUFFERS
            | EGL_SURFACE_TYPE
            | EGL_RENDERABLE_TYPE
            | EGL_CONFORMANT
            | EGL_CONFIG_ID
            | EGL_CONFIG_CAVEAT
            | EGL_COLOR_BUFFER_TYPE
            | EGL_BIND_TO_TEXTURE_RGB
            | EGL_BIND_TO_TEXTURE_RGBA
            | EGL_LEVEL
            | EGL_MATCH_NATIVE_PIXMAP
            | EGL_MAX_SWAP_INTERVAL
            | EGL_MIN_SWAP_INTERVAL
            | EGL_NATIVE_RENDERABLE
            | EGL_NATIVE_VISUAL_ID
            | EGL_NATIVE_VISUAL_TYPE
            | EGL_TRANSPARENT_TYPE
            | EGL_TRANSPARENT_RED_VALUE
            | EGL_TRANSPARENT_GREEN_VALUE
            | EGL_TRANSPARENT_BLUE_VALUE
    )
}

/// Whether `want` is one of the values EGL 1.5 table 3.4 defines for `key`.
/// The attributes not named here take any integer (EGL_DONT_CARE is already
/// out by the time this is asked).
fn config_value_ok(key: EGLint, want: EGLint) -> bool {
    match key {
        EGL_BIND_TO_TEXTURE_RGB | EGL_BIND_TO_TEXTURE_RGBA | EGL_NATIVE_RENDERABLE => {
            want == EGL_TRUE || want == EGL_FALSE
        }
        EGL_COLOR_BUFFER_TYPE => want == EGL_RGB_BUFFER || want == EGL_LUMINANCE_BUFFER,
        EGL_TRANSPARENT_TYPE => want == EGL_NONE || want == EGL_TRANSPARENT_RGB,
        EGL_CONFIG_CAVEAT => matches!(want, EGL_NONE | EGL_SLOW_CONFIG | EGL_NON_CONFORMANT_CONFIG),
        _ => true,
    }
}

/// Which colour components the caller asked for by a size that is neither 0
/// nor EGL_DONT_CARE. EGL 1.5 3.4.1 rule 3 sums only those.
///
/// # Safety
/// `attrib_list` is null or an EGL_NONE-terminated attribute list.
unsafe fn requested_color_bits(attrib_list: *const EGLint) -> [bool; 4] {
    let mut want = [false; 4];

    if attrib_list.is_null() {
        return want;
    }
    let mut p = attrib_list;
    loop {
        // SAFETY: the list is EGL_NONE-terminated and `p` has not passed
        // the terminator, so this name is inside it.
        let a = unsafe { p.read_unaligned() };
        if a == EGL_NONE {
            /* the terminator is one element: there is no value behind it */
            return want;
        }
        // SAFETY: a name other than EGL_NONE has its value behind it.
        let v = unsafe { p.add(1).read_unaligned() };
        /* the list is a map: a size given twice takes its later value, so
         * the assignment is not conditional on this one asking for bits */
        let asked = v != 0 && v != EGL_DONT_CARE;
        match a {
            EGL_RED_SIZE => want[0] = asked,
            EGL_GREEN_SIZE => want[1] = asked,
            EGL_BLUE_SIZE => want[2] = asked,
            EGL_ALPHA_SIZE => want[3] = asked,
            _ => {}
        }
        // SAFETY: as above; the terminator is checked before the step.
        p = unsafe { p.add(2) };
    }
}

/// The tuple EGL 1.5 3.4.1 sorts by, smallest first. Every rule that says
/// "larger first" is negated here so that one comparison serves.
///
/// Rules 9 (EGL_NATIVE_VISUAL_TYPE) and 10 are implementation defined and
/// EGL_NONE for every config here, so the config id decides the rest.
fn config_sort_key(c: EGLConfig, want: [bool; 4]) -> (i32, i32, i32, i32, i32, i32, i32, i32, i32, i32) {
    let at = |a: EGLint| config_attrib(c, a).unwrap_or(0);
    /* rule 1: EGL_NONE before the caveats */
    let caveat = match at(EGL_CONFIG_CAVEAT) {
        EGL_NONE => 0,
        EGL_SLOW_CONFIG => 1,
        _ => 2,
    };
    /* rule 3: larger total of the requested components' bits, first */
    let sizes = [EGL_RED_SIZE, EGL_GREEN_SIZE, EGL_BLUE_SIZE, EGL_ALPHA_SIZE];
    let bits: i32 = sizes.iter().zip(want).filter(|(_, w)| *w).map(|(a, _)| at(*a)).sum();
    (
        caveat,
        at(EGL_COLOR_BUFFER_TYPE),
        -bits,
        at(EGL_BUFFER_SIZE),
        at(EGL_SAMPLE_BUFFERS),
        at(EGL_SAMPLES),
        at(EGL_DEPTH_SIZE),
        at(EGL_STENCIL_SIZE),
        at(EGL_ALPHA_MASK_SIZE),
        at(EGL_CONFIG_ID),
    )
}

#[no_mangle]
pub unsafe extern "C" fn eglChooseConfig(
    dpy: EGLDisplay,
    attrib_list: *const EGLint,
    configs: *mut EGLConfig,
    config_size: EGLint,
    num_config: *mut EGLint,
) -> EGLBoolean {
    egl_entry!();
    gl_trace!(eglChooseConfig);
    state().error = EGL_SUCCESS;

    let e = state();
    let mut n: EGLint = 0;

    if !e.valid_display(dpy) {
        return FALSE;
    }
    if num_config.is_null() {
        return e.fail(EGL_BAD_PARAMETER);
    }
    let mut matched = [CONFIG_1X; NUM_CONFIGS as usize];
    let mut nmatched = 0usize;
    for &cfg in offered_configs() {
        // SAFETY: attrib_list is null or EGL_NONE-terminated (EGL contract).
        match unsafe { config_matches(cfg, attrib_list) } {
            Ok(true) => {
                matched[nmatched] = cfg;
                nmatched += 1;
            }
            Ok(false) => {}
            Err(()) => return e.fail(EGL_BAD_ATTRIBUTE),
        }
    }
    /* EGL 1.5 3.4.1's sort order, which this used to approximate with a fixed
     * list. dEQP checks it directly ("got correct configs but in invalid
     * order"), and rule 3 is the reason a fixed list cannot be right: the
     * colour bits are compared only for the components the caller asked for
     * by a non-zero, non-DONT_CARE size, so the order depends on the
     * attribute list. */
    // SAFETY: attrib_list is null or EGL_NONE-terminated (EGL contract).
    let want = unsafe { requested_color_bits(attrib_list) };
    let key = |c: EGLConfig| config_sort_key(c, want);
    let m = &mut matched[..nmatched];
    for i in 1..m.len() {
        let mut j = i;
        while j > 0 && key(m[j - 1]) > key(m[j]) {
            m.swap(j - 1, j);
            j -= 1;
        }
    }
    for &cfg in m.iter() {
        if !configs.is_null() && n < config_size {
            // SAFETY: configs is valid for config_size entries and
            // n < config_size (no alignment assumed).
            unsafe { configs.add(n as usize).write_unaligned(cfg) };
        }
        n += 1;
    }
    // SAFETY: num_config is non-null and valid for a write (EGL contract),
    // not necessarily aligned.
    unsafe {
        num_config.write_unaligned(if configs.is_null() || n < config_size { n } else { config_size.max(0) });
    }
    TRUE
}

/* ---- surfaces ------------------------------------------------------------ */

#[no_mangle]
pub unsafe extern "C" fn eglCreateWindowSurface(
    dpy: EGLDisplay,
    config: EGLConfig,
    win: EGLNativeWindowType,
    _attrib_list: *const EGLint, /* EGL_RENDER_BUFFER: always single-buffered */
) -> EGLSurface {
    egl_entry!();
    state().error = EGL_SUCCESS;
    create_window_surface(state(), dpy, config, win)
}

/// # Safety
/// `win` must be null or point at a native window description.
unsafe fn create_window_surface(
    e: &mut EglState,
    dpy: EGLDisplay,
    config: EGLConfig,
    win: EGLNativeWindowType,
) -> EGLSurface {
    gl_trace!(eglCreateWindowSurface); /* eglCreatePlatformWindowSurface enters it too */
    if !e.valid_display(dpy) {
        return NO_SURFACE;
    }
    if !valid_config(config) {
        e.error = EGL_BAD_CONFIG;
        return NO_SURFACE;
    }
    // SAFETY: win is null or a valid native window description (EGL contract).
    let win = unsafe { win.as_ref() }.copied();
    /* the scan-order flag is a window's alone: strip it before the buffer checks */
    let bottom_up = win.is_some_and(|w| w.format & GLES_NATIVE_WINDOW_BOTTOM_UP != 0);
    let win = win.map(|w| GlesNativeWindow { format: w.format & !GLES_NATIVE_WINDOW_BOTTOM_UP, ..w });
    let Some((win, (format, pitch))) = win.and_then(|w| Some((w, native_window_pitch(&w)?))) else {
        e.error = EGL_BAD_NATIVE_WINDOW;
        return NO_SURFACE;
    };
    /* the config's colour depth must be the window's (EGL 1.5 3.5.1) */
    if (config == CONFIG_565) != (win.format == GLES_NATIVE_FORMAT_RGB565) {
        e.error = EGL_BAD_MATCH;
        return NO_SURFACE;
    }
    let (map_phys, mmu_map) = osal::ops().map_or((None, None), |o| (o.map_phys, o.mmu_map));
    if map_phys.is_none() && mmu_map.is_none() {
        e.error = EGL_BAD_NATIVE_WINDOW;
        return NO_SURFACE;
    }
    let Some(i) = e.alloc_surface_slot() else {
        e.error = EGL_BAD_ALLOC;
        return NO_SURFACE;
    };
    let bytes = native_buffer_bytes(&win);
    let heap = backend::heap_base(GLES_HEAP_DATA);
    let s = &mut e.surf[i];
    if let Some(map_phys) = map_phys {
        /* the window's pages at a fresh device VA: no pool memory used */
        // SAFETY: `s.mem` is a valid exclusive reference for the call.
        if unsafe { map_phys(heap, win.phys, bytes, 0, &mut s.mem) } != 0 {
            e.error = EGL_BAD_NATIVE_WINDOW;
            return NO_SURFACE;
        }
    } else if let Some(mmu_map) = mmu_map {
        /* a GPU VA range from the render pool, repointed at the window's
         * pages (the pool pages behind the VA stay unused) */
        if osal::gpu_alloc(heap, bytes, OSAL_MEM_DATA, &mut s.mem) != 0 {
            e.error = EGL_BAD_ALLOC;
            return NO_SURFACE;
        }
        // SAFETY: the VA range was just allocated with this size.
        if unsafe { mmu_map(s.mem.gpu, win.phys, bytes, 0) } != 0 {
            e.error = EGL_BAD_NATIVE_WINDOW;
            return NO_SURFACE;
        }
        s.mem.cpu = core::ptr::null_mut(); /* the pool copy is not the window; no CPU view */
        s.mem.phys = win.phys;
    }
    s.used = true;
    s.window = true;
    s.bottom_up = bottom_up;
    s.config_id = config_id(config);
    s.samples = config_samples(config);
    s.format = format;
    s.width = win.width;
    s.height = win.height;
    /* 0: the packed pitch, as every surface but a view */
    s.pitch = if pitch == context::tex_pitch(win.width) { 0 } else { pitch };
    e.surface_handle(i)
}

#[no_mangle]
pub unsafe extern "C" fn eglCreatePlatformWindowSurface(
    dpy: EGLDisplay,
    config: EGLConfig,
    native_window: *mut c_void,
    _attrib_list: *const EGLAttrib,
) -> EGLSurface {
    egl_entry!();
    gl_trace!(eglCreatePlatformWindowSurface);
    state().error = EGL_SUCCESS;
    create_window_surface(state(), dpy, config, native_window as EGLNativeWindowType)
}

/// The attribute names eglCreatePbufferSurface accepts, EGL 1.5 3.5.1.
///
/// `Err` carries the attribute's error: an unknown name or a value outside
/// the attribute's set is EGL_BAD_ATTRIBUTE, a negative EGL_WIDTH or
/// EGL_HEIGHT is EGL_BAD_PARAMETER, and a request this EGL cannot honour
/// (render-to-texture, sRGB, OpenVG) is EGL_BAD_MATCH.
///
/// The walk stops at the first bad *name* without reading its value: dEQP
/// passes the one-element list {0xffffffff}, with no EGL_NONE after it.
///
/// # Safety
/// `list` is null or EGL_NONE-terminated, the EGL contract for the caller.
unsafe fn check_pbuffer_attribs(list: *const EGLint) -> Result<(), EGLint> {
    if list.is_null() {
        return Ok(());
    }
    let mut a = list;
    /* the application's array: read without assuming alignment */
    loop {
        let name = a.read_unaligned();
        if name == EGL_NONE {
            break;
        }
        let value = || a.add(1).read_unaligned();
        match name {
            EGL_WIDTH | EGL_HEIGHT => {
                if value() < 0 {
                    return Err(EGL_BAD_PARAMETER);
                }
            }
            /* the size is refused outright, so nothing is clamped to it */
            EGL_LARGEST_PBUFFER | EGL_MIPMAP_TEXTURE => match value() {
                EGL_TRUE | EGL_FALSE => {}
                _ => return Err(EGL_BAD_ATTRIBUTE),
            },
            EGL_TEXTURE_FORMAT => match value() {
                EGL_NO_TEXTURE => {}
                /* EGL_BIND_TO_TEXTURE_RGB(A) is EGL_FALSE on every config */
                EGL_TEXTURE_RGB | EGL_TEXTURE_RGBA => return Err(EGL_BAD_MATCH),
                _ => return Err(EGL_BAD_ATTRIBUTE),
            },
            EGL_TEXTURE_TARGET => match value() {
                EGL_NO_TEXTURE => {}
                EGL_TEXTURE_2D => return Err(EGL_BAD_MATCH),
                _ => return Err(EGL_BAD_ATTRIBUTE),
            },
            EGL_GL_COLORSPACE => match value() {
                EGL_GL_COLORSPACE_LINEAR => {}
                EGL_GL_COLORSPACE_SRGB => return Err(EGL_BAD_MATCH), /* no sRGB encoding */
                _ => return Err(EGL_BAD_ATTRIBUTE),
            },
            EGL_VG_COLORSPACE => match value() {
                EGL_VG_COLORSPACE_sRGB => {}
                EGL_VG_COLORSPACE_LINEAR => return Err(EGL_BAD_MATCH), /* no OpenVG */
                _ => return Err(EGL_BAD_ATTRIBUTE),
            },
            EGL_VG_ALPHA_FORMAT => match value() {
                EGL_VG_ALPHA_FORMAT_NONPRE => {}
                EGL_VG_ALPHA_FORMAT_PRE => return Err(EGL_BAD_MATCH), /* no OpenVG */
                _ => return Err(EGL_BAD_ATTRIBUTE),
            },
            _ => return Err(EGL_BAD_ATTRIBUTE),
        }
        a = a.add(2);
    }
    /* EGL 1.5 3.5.1: the format and the target are both EGL_NO_TEXTURE or
     * neither is. Only the first pair is reachable here, but a list naming
     * one of the two alone still has to be caught. */
    let format = attrib_of(list, EGL_TEXTURE_FORMAT, EGL_NO_TEXTURE);
    let target = attrib_of(list, EGL_TEXTURE_TARGET, EGL_NO_TEXTURE);
    if (format == EGL_NO_TEXTURE) != (target == EGL_NO_TEXTURE) {
        return Err(EGL_BAD_MATCH);
    }
    Ok(())
}

#[no_mangle]
pub unsafe extern "C" fn eglCreatePbufferSurface(
    dpy: EGLDisplay,
    config: EGLConfig,
    attrib_list: *const EGLint,
) -> EGLSurface {
    egl_entry!();
    gl_trace!(eglCreatePbufferSurface);
    let e = state();
    e.error = EGL_SUCCESS;
    if !e.valid_display(dpy) {
        return NO_SURFACE;
    }
    if !valid_config(config) {
        e.error = EGL_BAD_CONFIG;
        return NO_SURFACE;
    }
    // SAFETY: attrib_list is null or EGL_NONE-terminated (EGL contract).
    if let Err(err) = unsafe { check_pbuffer_attribs(attrib_list) } {
        e.error = err;
        return NO_SURFACE;
    }
    /* the size is read after the walk above, which is what establishes that
     * the list ends where it says it does */
    // SAFETY: as above.
    let (w, h) = unsafe { (attrib_of(attrib_list, EGL_WIDTH, 0), attrib_of(attrib_list, EGL_HEIGHT, 0)) };
    /* what is left is a size this EGL has no storage for: the EGL default
     * of 0 x 0 has nothing to render into, and 4096 is the limit
     * EGL_MAX_PBUFFER_WIDTH / _HEIGHT report */
    if w <= 0 || h <= 0 || w > 4096 || h > 4096 {
        e.error = EGL_BAD_ATTRIBUTE;
        return NO_SURFACE;
    }
    let Some(i) = e.alloc_surface_slot() else {
        e.error = EGL_BAD_ALLOC;
        return NO_SURFACE;
    };
    let s = &mut e.surf[i];
    /* the config's own format, at the pair-padded pitch the PBE writes and
     * glReadPixels reads (an odd width is one pixel wider per row) */
    let format = config_surf_format(config);
    let bytes = context::surface_bytes(w as u32, h as u32, gles_surf_bytes_per_pixel(format));
    if osal::gpu_alloc(backend::heap_base(GLES_HEAP_DATA), bytes, OSAL_MEM_DATA, &mut s.mem) != 0 {
        e.error = EGL_BAD_ALLOC;
        return NO_SURFACE;
    }
    /* pool memory: no earlier user's pixels before the first clear */
    context::clear_storage(&mut s.mem);
    s.used = true;
    s.config_id = config_id(config);
    s.samples = config_samples(config);
    s.format = format;
    s.width = w as u32;
    s.height = h as u32;
    e.surface_handle(i)
}

#[no_mangle]
pub unsafe extern "C" fn eglCreatePixmapSurface(
    dpy: EGLDisplay,
    config: EGLConfig,
    _pixmap: EGLNativePixmapType,
    _attrib_list: *const EGLint,
) -> EGLSurface {
    egl_entry!();
    create_pixmap_surface(dpy, config)
}

/// eglCreatePixmapSurface, under the lock its callers hold. There are no
/// pixmap surfaces here; the platform entry point says the same thing.
fn create_pixmap_surface(dpy: EGLDisplay, config: EGLConfig) -> EGLSurface {
    gl_trace!(eglCreatePixmapSurface);
    let e = state();
    e.error = EGL_SUCCESS;
    if e.valid_display(dpy) {
        /* the config is judged before the surface type it cannot provide:
         * a config that is no config is EGL_BAD_CONFIG whatever is asked
         * of it */
        e.error = if valid_config(config) {
            EGL_BAD_MATCH /* EGL_PIXMAP_BIT is in no config's EGL_SURFACE_TYPE */
        } else {
            EGL_BAD_CONFIG
        };
    }
    NO_SURFACE
}

#[no_mangle]
pub unsafe extern "C" fn eglCreatePlatformPixmapSurface(
    dpy: EGLDisplay,
    config: EGLConfig,
    native_pixmap: *mut c_void,
    _attrib_list: *const EGLAttrib,
) -> EGLSurface {
    egl_entry!();
    gl_trace!(eglCreatePlatformPixmapSurface);
    state().error = EGL_SUCCESS;
    let _ = native_pixmap;
    create_pixmap_surface(dpy, config)
}

#[no_mangle]
pub unsafe extern "C" fn eglCreatePbufferFromClientBuffer(
    dpy: EGLDisplay,
    _buftype: EGLenum,
    _buffer: EGLClientBuffer,
    _config: EGLConfig,
    _attrib_list: *const EGLint,
) -> EGLSurface {
    egl_entry!();
    gl_trace!(eglCreatePbufferFromClientBuffer);
    let e = state();
    e.error = EGL_SUCCESS;
    if e.valid_display(dpy) {
        e.error = EGL_BAD_PARAMETER;
    }
    NO_SURFACE
}

/// eglDestroySurface (traced here: eglTerminate enters it per surface).
fn destroy_surface(e: &mut EglState, dpy: EGLDisplay, surface: EGLSurface) -> EGLBoolean {
    gl_trace!(eglDestroySurface);
    if !e.valid_display(dpy) {
        return FALSE;
    }
    let Some(i) = e.lookup_surface(surface) else {
        return e.fail(EGL_BAD_SURFACE);
    };
    if e.cur_draw == Some(i) {
        make_current(e, dpy, NO_SURFACE, NO_SURFACE, NO_CONTEXT);
    }
    /* pbuffer memory: the OSAL frees it when it can; a window mapping is
     * left (the pool VA range is not reclaimable without an unmap) */
    let s = &mut e.surf[i];
    /* its own depth/stencil (`GlesDraw::depth_id`: bit 31 and the colour
     * buffer's page) goes with it */
    if !s.mem.impl_.is_null() {
        backend::attachment_gone(0x8000_0000 | (s.mem.gpu >> 12) as u32);
    }
    if !s.window && !s.mem.impl_.is_null() {
        osal::gpu_free(&mut s.mem);
    }
    *s = EglSurface::ZERO;
    TRUE
}

#[no_mangle]
pub unsafe extern "C" fn eglDestroySurface(dpy: EGLDisplay, surface: EGLSurface) -> EGLBoolean {
    egl_entry!();
    state().error = EGL_SUCCESS;
    destroy_surface(state(), dpy, surface)
}

/// EGL_RENDER_BUFFER for a surface: EGL 1.5 table 3.5 gives a pbuffer
/// EGL_BACK_BUFFER (dEQP checks eglQueryContext against the surface kind),
/// and a window surface here is single-buffered -- it is rendered in place.
fn render_buffer_of(s: &EglSurface) -> EGLint {
    if s.window {
        EGL_SINGLE_BUFFER
    } else {
        EGL_BACK_BUFFER
    }
}

#[no_mangle]
pub unsafe extern "C" fn eglQuerySurface(
    dpy: EGLDisplay,
    surface: EGLSurface,
    attribute: EGLint,
    value: *mut EGLint,
) -> EGLBoolean {
    egl_entry!();
    gl_trace!(eglQuerySurface);
    let e = state();
    e.error = EGL_SUCCESS;
    if !e.valid_display(dpy) {
        return FALSE;
    }
    let Some(i) = e.lookup_surface(surface) else {
        return e.fail(EGL_BAD_SURFACE);
    };
    if value.is_null() {
        return e.fail(EGL_BAD_PARAMETER);
    }
    let s = &e.surf[i];
    let v = match attribute {
        EGL_WIDTH => s.width as EGLint,
        EGL_HEIGHT => s.height as EGLint,
        EGL_CONFIG_ID => s.config_id as EGLint,
        EGL_SWAP_BEHAVIOR => {
            if s.swap_destroyed {
                EGL_BUFFER_DESTROYED
            } else {
                EGL_BUFFER_PRESERVED
            }
        }
        EGL_RENDER_BUFFER => render_buffer_of(s),
        EGL_MULTISAMPLE_RESOLVE => EGL_MULTISAMPLE_RESOLVE_DEFAULT,
        EGL_VG_ALPHA_FORMAT => EGL_VG_ALPHA_FORMAT_NONPRE,
        EGL_VG_COLORSPACE => EGL_VG_COLORSPACE_sRGB,
        EGL_HORIZONTAL_RESOLUTION | EGL_VERTICAL_RESOLUTION | EGL_PIXEL_ASPECT_RATIO => EGL_UNKNOWN,
        EGL_LARGEST_PBUFFER => EGL_FALSE,
        EGL_TEXTURE_FORMAT => EGL_NO_TEXTURE,
        EGL_TEXTURE_TARGET => EGL_NO_TEXTURE,
        EGL_MIPMAP_TEXTURE => EGL_FALSE,
        EGL_MIPMAP_LEVEL => 0,
        _ => return e.fail(EGL_BAD_ATTRIBUTE),
    };
    // SAFETY: value is non-null and valid for a write (EGL contract), not
    // necessarily aligned.
    unsafe { value.write_unaligned(v) };
    TRUE
}

#[no_mangle]
pub unsafe extern "C" fn eglSurfaceAttrib(
    dpy: EGLDisplay,
    surface: EGLSurface,
    attribute: EGLint,
    value: EGLint,
) -> EGLBoolean {
    egl_entry!();
    gl_trace!(eglSurfaceAttrib);
    let e = state();
    e.error = EGL_SUCCESS;
    if !e.valid_display(dpy) {
        return FALSE;
    }
    let Some(i) = e.lookup_surface(surface) else {
        return e.fail(EGL_BAD_SURFACE);
    };
    /* EGL 1.5 3.5.6: a value the surface's config cannot do is
     * EGL_BAD_MATCH, not success. EGL_MULTISAMPLE_RESOLVE_BOX_BIT is in no
     * config's EGL_SURFACE_TYPE -- there is no box filter here -- so the
     * box resolve is refused; EGL_SWAP_BEHAVIOR_PRESERVED_BIT is in all of
     * them, so both swap behaviours are taken. */
    match attribute {
        EGL_SWAP_BEHAVIOR => match value {
            EGL_BUFFER_PRESERVED | EGL_BUFFER_DESTROYED => {
                e.surf[i].swap_destroyed = value == EGL_BUFFER_DESTROYED;
                TRUE
            }
            _ => e.fail(EGL_BAD_PARAMETER),
        },
        EGL_MULTISAMPLE_RESOLVE => match value {
            EGL_MULTISAMPLE_RESOLVE_DEFAULT => TRUE,
            EGL_MULTISAMPLE_RESOLVE_BOX => e.fail(EGL_BAD_MATCH),
            _ => e.fail(EGL_BAD_PARAMETER),
        },
        EGL_MIPMAP_LEVEL => TRUE,
        _ => e.fail(EGL_BAD_ATTRIBUTE),
    }
}

#[no_mangle]
pub unsafe extern "C" fn eglBindTexImage(dpy: EGLDisplay, surface: EGLSurface, _buffer: EGLint) -> EGLBoolean {
    egl_entry!();
    bind_tex_image(dpy, surface)
}

/// eglBindTexImage, under the lock its callers hold. There are no
/// render-to-texture pbuffers here, so both it and eglReleaseTexImage say
/// so, and they say it the same way.
fn bind_tex_image(dpy: EGLDisplay, surface: EGLSurface) -> EGLBoolean {
    gl_trace!(eglBindTexImage);
    let e = state();
    e.error = EGL_SUCCESS;
    if !e.valid_display(dpy) {
        return FALSE;
    }
    let err = if e.lookup_surface(surface).is_some() { EGL_BAD_MATCH } else { EGL_BAD_SURFACE };
    e.fail(err)
}

#[no_mangle]
pub unsafe extern "C" fn eglReleaseTexImage(dpy: EGLDisplay, surface: EGLSurface, _buffer: EGLint) -> EGLBoolean {
    egl_entry!();
    gl_trace!(eglReleaseTexImage);
    state().error = EGL_SUCCESS;
    bind_tex_image(dpy, surface)
}

#[no_mangle]
pub unsafe extern "C" fn eglCopyBuffers(
    dpy: EGLDisplay,
    surface: EGLSurface,
    _target: EGLNativePixmapType,
) -> EGLBoolean {
    egl_entry!();
    gl_trace!(eglCopyBuffers);
    let e = state();
    e.error = EGL_SUCCESS;
    if !e.valid_display(dpy) {
        return FALSE;
    }
    let err = if e.lookup_surface(surface).is_some() { EGL_BAD_NATIVE_PIXMAP } else { EGL_BAD_SURFACE };
    e.fail(err)
}

/* ---- contexts ------------------------------------------------------------ */

/// The attribute names eglCreateContext accepts, EGL 1.5 3.7.1.
///
/// `Err` carries the error the offending attribute earns: an unknown name, a
/// name that is OpenGL-only (this EGL has OpenGL ES contexts alone), or a
/// value outside the attribute's set is EGL_BAD_ATTRIBUTE; a well-formed
/// request this EGL cannot honour is EGL_BAD_MATCH.
///
/// The walk stops at the first bad *name* without reading its value: dEQP
/// passes the one-element list {0xffffffff}, with no EGL_NONE after it.
///
/// # Safety
/// `list` is null or EGL_NONE-terminated, the EGL contract for the caller.
unsafe fn check_context_attribs(list: *const EGLint) -> Result<(), EGLint> {
    if list.is_null() {
        return Ok(());
    }
    let mut a = list;
    /* the application's array: read without assuming alignment */
    loop {
        let name = a.read_unaligned();
        if name == EGL_NONE {
            return Ok(());
        }
        match name {
            /* EGL_CONTEXT_CLIENT_VERSION is EGL_CONTEXT_MAJOR_VERSION */
            EGL_CONTEXT_MAJOR_VERSION | EGL_CONTEXT_MINOR_VERSION => {}
            /* EGL_KHR_create_context's flags: debug is honoured as the
             * attribute is; robust access this EGL cannot give; forward
             * compatibility is OpenGL-only, an error for an ES context. */
            EGL_CONTEXT_FLAGS_KHR => {
                let f = a.add(1).read_unaligned();
                const DEBUG: EGLint = 0x1;
                const ROBUST: EGLint = 0x4;
                if f & !(DEBUG | ROBUST) != 0 {
                    return Err(EGL_BAD_ATTRIBUTE);
                }
                if f & ROBUST != 0 {
                    return Err(EGL_BAD_MATCH);
                }
            }
            EGL_CONTEXT_OPENGL_DEBUG => match a.add(1).read_unaligned() {
                /* a debug context need not say more than this one does */
                EGL_TRUE | EGL_FALSE => {}
                _ => return Err(EGL_BAD_ATTRIBUTE),
            },
            EGL_CONTEXT_OPENGL_ROBUST_ACCESS => match a.add(1).read_unaligned() {
                EGL_FALSE => {}
                EGL_TRUE => return Err(EGL_BAD_MATCH), /* no robust buffer access */
                _ => return Err(EGL_BAD_ATTRIBUTE),
            },
            EGL_CONTEXT_OPENGL_RESET_NOTIFICATION_STRATEGY => match a.add(1).read_unaligned() {
                EGL_NO_RESET_NOTIFICATION => {}
                EGL_LOSE_CONTEXT_ON_RESET => return Err(EGL_BAD_MATCH), /* contexts are never lost */
                _ => return Err(EGL_BAD_ATTRIBUTE),
            },
            /* OpenGL-only attributes, and everything unknown */
            _ => return Err(EGL_BAD_ATTRIBUTE),
        }
        a = a.add(2);
    }
}

#[no_mangle]
pub unsafe extern "C" fn eglCreateContext(
    dpy: EGLDisplay,
    config: EGLConfig,
    share_context: EGLContext,
    attrib_list: *const EGLint,
) -> EGLContext {
    egl_entry!();
    gl_trace!(eglCreateContext);
    let e = state();
    e.error = EGL_SUCCESS;
    if !e.valid_display(dpy) {
        return NO_CONTEXT;
    }
    if !valid_config(config) {
        e.error = EGL_BAD_CONFIG;
        return NO_CONTEXT;
    }
    // SAFETY: attrib_list is null or EGL_NONE-terminated (EGL contract).
    if let Err(err) = unsafe { check_context_attribs(attrib_list) } {
        e.error = err;
        return NO_CONTEXT;
    }
    /* EGL 1.5 3.7.1: the new context shares the objects of `share_context`.
     * A handle that is not a context of this display is EGL_BAD_CONTEXT. */
    let share = if share_context == NO_CONTEXT {
        context::GROUP_NONE
    } else {
        let Some(si) = e.lookup_context(share_context) else {
            e.error = EGL_BAD_CONTEXT;
            return NO_CONTEXT;
        };
        /* The group is made when a second context needs it, or when the
         * first eglMakeCurrent makes a GL context; a context that has been
         * neither shared with nor current holds none, which is what it
         * held before share groups existed. This is the first case: the
         * context being shared with gets the group too, and a handle on
         * it. */
        if e.ctx[si].group == context::GROUP_NONE {
            let g = context::gl_group_alloc(context::GROUP_NONE);
            if g == context::GROUP_NONE {
                e.error = EGL_BAD_ALLOC;
                return NO_CONTEXT;
            }
            e.ctx[si].group = g;
        }
        e.ctx[si].group
    };
    if e.api != 0 && e.api != EGL_OPENGL_ES_API as EGLenum {
        e.error = EGL_BAD_MATCH;
        return NO_CONTEXT;
    }
    // SAFETY: attrib_list is null or EGL_NONE-terminated (EGL contract).
    let version = unsafe { attrib_of(attrib_list, EGL_CONTEXT_CLIENT_VERSION, 1) };
    /* One kind of context: OpenGL ES 3.0, which is what an ES 2.0 request
     * gets too (EGL allows a backwards-compatible later version; Qt reads
     * the real version back from GL_VERSION). */
    if version != 2 && version != 3 {
        e.error = EGL_BAD_MATCH;
        return NO_CONTEXT;
    }
    if let Some(i) = e.ctx.iter().position(|c| !c.used) {
        /* a handle on the group being shared, so that it outlives the
         * context that made it; GROUP_NONE until this context is current */
        let group = if share == context::GROUP_NONE { share } else { context::gl_group_alloc(share) };
        if share != context::GROUP_NONE && group == context::GROUP_NONE {
            e.error = EGL_BAD_ALLOC;
            return NO_CONTEXT;
        }
        e.ctx[i] = EglContext { used: true, gl: core::ptr::null_mut(), config_id: config_id(config), group };
        return e.context_handle(i);
    }
    e.error = EGL_BAD_ALLOC; /* MAX_CONTEXTS contexts exist already */
    NO_CONTEXT
}

/// eglDestroyContext (traced here: eglTerminate enters it per context).
fn destroy_context(e: &mut EglState, dpy: EGLDisplay, ctx: EGLContext) -> EGLBoolean {
    gl_trace!(eglDestroyContext);
    if !e.valid_display(dpy) {
        return FALSE;
    }
    let Some(i) = e.lookup_context(ctx) else {
        return e.fail(EGL_BAD_CONTEXT);
    };
    if e.cur_ctx == Some(i) {
        make_current(e, dpy, NO_SURFACE, NO_SURFACE, NO_CONTEXT);
    }
    let (gl, group) = (e.ctx[i].gl, e.ctx[i].group);
    if !gl.is_null() {
        /* the context's EGLImage siblings drop their hold on the images'
         * storage through the state this call already holds */
        // SAFETY: gl came from gl_context_create_for_surface and has not
        // been destroyed; no reference to the GL context is live here.
        unsafe { context::destroy_with(gl, &mut |m| image_storage_release_in(e, m)) };
    }
    /* and the handle on the share group this context took when it was
     * created: the objects go when the last of the group's contexts does */
    context::group_release_with(group, &mut |m| image_storage_release_in(e, m));
    e.ctx[i] = EglContext::ZERO;
    TRUE
}

#[no_mangle]
pub unsafe extern "C" fn eglDestroyContext(dpy: EGLDisplay, ctx: EGLContext) -> EGLBoolean {
    egl_entry!();
    state().error = EGL_SUCCESS;
    destroy_context(state(), dpy, ctx)
}

/// eglMakeCurrent (traced here: the destroy paths and eglTerminate enter it).
fn make_current(e: &mut EglState, dpy: EGLDisplay, draw: EGLSurface, read: EGLSurface, ctx: EGLContext) -> EGLBoolean {
    gl_trace!(eglMakeCurrent);
    if dpy != the_display() {
        return e.fail(EGL_BAD_DISPLAY);
    }
    if ctx == NO_CONTEXT {
        if draw != NO_SURFACE || read != NO_SURFACE {
            return e.fail(EGL_BAD_MATCH);
        }
        if e.cur_ctx.is_some() {
            flush_pending_clear();
            backend::flush();
            backend::finish();
            // SAFETY: no reference to the current GL context is live.
            unsafe { context::gl_context_set_current(core::ptr::null_mut()) };
        }
        e.cur_ctx = None;
        e.cur_draw = None;
        context::owner_release();
        return TRUE;
    }
    if !e.inited {
        return e.fail(EGL_NOT_INITIALIZED);
    }
    let Some(ci) = e.lookup_context(ctx) else {
        return e.fail(EGL_BAD_CONTEXT);
    };
    /* EGL 1.5 3.7.3 checks the handles first: a surface that is not a
     * surface is EGL_BAD_SURFACE, whichever of the two it is. Only then does
     * this library's own restriction apply. Checking the restriction first
     * answered EGL_BAD_MATCH for a bad read surface. */
    if draw == NO_SURFACE || read == NO_SURFACE {
        /* no surfaceless context here */
        return e.fail(EGL_BAD_MATCH);
    }
    let (Some(di), Some(ri)) = (e.lookup_surface(draw), e.lookup_surface(read)) else {
        return e.fail(EGL_BAD_SURFACE);
    };
    if di != ri {
        return e.fail(EGL_BAD_MATCH); /* one surface serves both */
    }
    let si = di;
    if let Some(cur) = e.cur_ctx {
        /* The frame recorded so far is submitted. Switching to another
         * context also waits for it (the contexts share the backend's
         * frame sets); moving the same context to another surface, the
         * double/triple-buffering case, must not, or the frame in flight
         * serialises the next one (gl_slint: 33 fps instead of GPU-bound). */
        flush_pending_clear();
        backend::flush();
        if ci != cur {
            backend::finish();
        }
    }
    let s = &mut e.surf[si];
    let (w, h, format, samples) = (s.width, s.height, s.format, s.samples);
    if e.ctx[ci].gl.is_null() {
        /* first use: the GL state starts from its defaults */
        // SAFETY: the surface descriptor is valid for the call; no reference
        // to a GL context is live (the one used for the flush is gone).
        let gl = unsafe { context::gl_context_create_for_surface(&mut s.mem, w, h, format, samples, e.ctx[ci].group) };
        e.ctx[ci].gl = gl;
        if gl.is_null() {
            return e.fail(EGL_BAD_ALLOC);
        }
        if e.ctx[ci].group == context::GROUP_NONE {
            /* the group the GL context just made: the EGLContext takes a
             * handle of its own, so a later eglCreateContext can share it
             * and so it reads back the same one every time */
            // SAFETY: `gl` is the context just created.
            let g = unsafe { (*gl).group };
            e.ctx[ci].group = context::gl_group_alloc(g);
        }
    } else {
        /* the context keeps its state, only the surface changes */
        // SAFETY: as above; e.ctx[ci].gl is a live GL context.
        let bound = unsafe {
            context::gl_context_set_current(e.ctx[ci].gl);
            context::gl_context_bind_surface(&mut s.mem, w, h, format, samples)
        };
        if bound != 0 {
            /* The bind refused, so the context is still pointing at whatever
             * surface it had. Returning TRUE here would leave the application
             * drawing into the old surface and reading back the new one, with
             * no error raised anywhere -- so this fails, and eglMakeCurrent
             * leaves the previously current context current (EGL 1.5 3.7.3). */
            // SAFETY: the previous context index, if any, is a live context.
            unsafe {
                context::gl_context_set_current(match e.cur_ctx {
                    Some(prev) => e.ctx[prev].gl,
                    None => core::ptr::null_mut(),
                })
            };
            return e.fail(EGL_BAD_SURFACE);
        }
    }
    /* a window that is a view into a wider buffer keeps its own row pitch
     * (0 restores the packed one for every other surface); its mapping
     * holds height rows of it by construction (native_buffer_bytes) */
    if context::set_default_pitch(e.surf[si].pitch) != 0 {
        return e.fail(EGL_BAD_SURFACE);
    }
    /* a window is scanned out top-down unless its display scans bottom row
     * first; a pbuffer is only read back. Both of those are drawn y-up as an
     * FBO is, and gl_FragCoord needs no mirroring */
    let y_up = !e.surf[si].window || e.surf[si].bottom_up;
    // SAFETY: e.ctx[ci].gl is the live context just created or bound.
    unsafe { (*e.ctx[ci].gl).default_y_up = y_up };
    e.cur_ctx = Some(ci);
    e.cur_draw = Some(si);
    context::owner_take();
    e.error = EGL_SUCCESS;
    TRUE
}

#[no_mangle]
pub unsafe extern "C" fn eglMakeCurrent(
    dpy: EGLDisplay,
    draw: EGLSurface,
    read: EGLSurface,
    ctx: EGLContext,
) -> EGLBoolean {
    egl_entry!();
    state().error = EGL_SUCCESS;
    make_current(state(), dpy, draw, read, ctx)
}

#[no_mangle]
pub unsafe extern "C" fn eglGetCurrentContext() -> EGLContext {
    egl_entry!();
    gl_trace!(eglGetCurrentContext);
    let e = state();
    e.error = EGL_SUCCESS;
    match e.cur_ctx {
        Some(i) => e.context_handle(i),
        None => NO_CONTEXT,
    }
}

#[no_mangle]
pub unsafe extern "C" fn eglGetCurrentSurface(readdraw: EGLint) -> EGLSurface {
    egl_entry!();
    gl_trace!(eglGetCurrentSurface);
    let e = state();
    e.error = EGL_SUCCESS;
    if readdraw != EGL_READ && readdraw != EGL_DRAW {
        e.error = EGL_BAD_PARAMETER;
        return NO_SURFACE;
    }
    match (e.cur_ctx, e.cur_draw) {
        (Some(_), Some(i)) => e.surface_handle(i),
        _ => NO_SURFACE,
    }
}

#[no_mangle]
pub unsafe extern "C" fn eglGetCurrentDisplay() -> EGLDisplay {
    egl_entry!();
    gl_trace!(eglGetCurrentDisplay);
    state().error = EGL_SUCCESS;
    if state().cur_ctx.is_some() {
        the_display()
    } else {
        NO_DISPLAY
    }
}

#[no_mangle]
pub unsafe extern "C" fn eglQueryContext(
    dpy: EGLDisplay,
    ctx: EGLContext,
    attribute: EGLint,
    value: *mut EGLint,
) -> EGLBoolean {
    egl_entry!();
    gl_trace!(eglQueryContext);
    let e = state();
    e.error = EGL_SUCCESS;
    if !e.valid_display(dpy) {
        return FALSE;
    }
    let Some(i) = e.lookup_context(ctx) else {
        return e.fail(EGL_BAD_CONTEXT);
    };
    if value.is_null() {
        return e.fail(EGL_BAD_PARAMETER);
    }
    let v = match attribute {
        EGL_CONFIG_ID => e.ctx[i].config_id as EGLint,
        EGL_CONTEXT_CLIENT_TYPE => EGL_OPENGL_ES_API,
        EGL_CONTEXT_CLIENT_VERSION => 3,
        EGL_RENDER_BUFFER => match e.cur_draw.filter(|_| e.cur_ctx == Some(i)) {
            /* EGL 1.5 table 3.5: rendering to a pbuffer is rendering to the
             * back buffer, whether or not anything is ever swapped out of
             * it; a window surface here is rendered in place */
            Some(si) => render_buffer_of(&e.surf[si]),
            None => EGL_NONE,
        },
        _ => return e.fail(EGL_BAD_ATTRIBUTE),
    };
    // SAFETY: value is non-null and valid for a write (EGL contract), not
    // necessarily aligned.
    unsafe { value.write_unaligned(v) };
    TRUE
}

/* ---- swap / sync --------------------------------------------------------- */

#[no_mangle]
pub unsafe extern "C" fn eglSwapBuffers(dpy: EGLDisplay, surface: EGLSurface) -> EGLBoolean {
    egl_entry!();
    gl_trace!(eglSwapBuffers);
    let e = state();
    e.error = EGL_SUCCESS;
    if !e.valid_display(dpy) {
        return FALSE;
    }
    let Some(i) = e.lookup_surface(surface) else {
        return e.fail(EGL_BAD_SURFACE);
    };
    if e.cur_ctx.is_none() || e.cur_draw != Some(i) {
        return e.fail(EGL_BAD_SURFACE);
    }
    /* single-buffered: the frame is complete in the surface when this returns */
    flush_pending_clear();
    /* EGL 1.5 3.10.1: the surface's ancillary buffers are undefined after
     * the swap; the backend may drop them before the frame is submitted.
     * Window surfaces only: for "a single-buffered window, pixmap, or
     * pbuffer surface, eglSwapBuffers has no effect" (the same paragraph),
     * so a pbuffer keeps its depth/stencil across a swap. */
    if e.surf[i].window {
        backend::surface_swapped(e.surf[i].mem.gpu);
    }
    /* Submit this frame and let it run; wait only for the frames before it,
     * so the caller may show the previous frame's buffer (double buffering)
     * while this one renders. eglWaitGL / glFinish wait for everything. */
    if backend::flush() != 0 {
        return e.fail(EGL_CONTEXT_LOST);
    }
    if backend::finish_previous() != 0 {
        return e.fail(EGL_CONTEXT_LOST);
    }
    /* Debug: the frame's kicks are done here, which is the only point at
     * which reading GPU-written texture storage says anything. */
    crate::gl::draw::debug_dump_cubes();
    /* EGL 1.5 3.10.3: a swap waits for `swap_interval` vertical periods
     * before it returns, so an application that draws in a loop runs at the
     * display's rate instead of as fast as the GPU goes. Without it a frame
     * costs whatever the GPU costs -- measured on the AM62P, 190 swaps of
     * 250 returned in about 0.1 ms -- and what reaches the panel is whatever
     * a scan-out catches, so motion is uneven however steady the renderer
     * is. A platform with no vertical sync answers -ENOTSUP and presentation
     * stays immediate, which is what every platform did before. */
    for _ in 0..e.swap_interval {
        if osal::wait_vblank(100_000) == -crate::util::ENOTSUP {
            break;
        }
    }
    TRUE
}

#[no_mangle]
pub unsafe extern "C" fn eglSwapInterval(dpy: EGLDisplay, interval: EGLint) -> EGLBoolean {
    egl_entry!();
    gl_trace!(eglSwapInterval);
    let e = state();
    e.error = EGL_SUCCESS;
    if !e.valid_display(dpy) {
        return FALSE;
    }
    if e.cur_ctx.is_none() {
        return e.fail(EGL_BAD_CONTEXT);
    }
    /* EGL 1.5 3.10.3: the interval is clamped to the config's min/max, and
     * 0 means "do not wait". With no vertical sync under us the value is
     * remembered and the wait below finds nothing to wait for, which is the
     * accept-and-ignore this returned before. */
    e.swap_interval = interval.clamp(0, EGL_MAX_SWAP_INTERVAL);
    TRUE
}

#[no_mangle]
pub unsafe extern "C" fn eglWaitGL() -> EGLBoolean {
    egl_entry!();
    wait_gl()
}

/// eglWaitGL, under the lock its callers hold. eglWaitClient is the same
/// wait: OpenGL ES is the only client API here.
fn wait_gl() -> EGLBoolean {
    gl_trace!(eglWaitGL);
    state().error = EGL_SUCCESS;
    if state().cur_ctx.is_some() {
        flush_pending_clear();
        backend::flush();
        backend::finish();
    }
    TRUE
}

#[no_mangle]
pub unsafe extern "C" fn eglWaitClient() -> EGLBoolean {
    egl_entry!();
    gl_trace!(eglWaitClient);
    state().error = EGL_SUCCESS;
    wait_gl()
}

#[no_mangle]
pub unsafe extern "C" fn eglWaitNative(engine: EGLint) -> EGLBoolean {
    egl_entry!();
    gl_trace!(eglWaitNative);
    state().error = EGL_SUCCESS;
    if engine != EGL_CORE_NATIVE_ENGINE {
        return state().fail(EGL_BAD_PARAMETER);
    }
    TRUE /* no native rendering to wait for */
}

#[no_mangle]
pub unsafe extern "C" fn eglCreateSync(dpy: EGLDisplay, type_: EGLenum, _attrib_list: *const EGLAttrib) -> EGLSync {
    egl_entry!();
    gl_trace!(eglCreateSync);
    let e = state();
    e.error = EGL_SUCCESS;
    if !e.valid_display(dpy) {
        return NO_SYNC;
    }
    if type_ != EGL_SYNC_FENCE as EGLenum {
        e.error = EGL_BAD_ATTRIBUTE;
        return NO_SYNC;
    }
    if e.cur_ctx.is_none() {
        e.error = EGL_BAD_MATCH;
        return NO_SYNC;
    }
    let Some(i) = e.syncs.iter().position(|s| !s.used) else {
        e.error = EGL_BAD_ALLOC;
        return NO_SYNC;
    };
    /* Prior commands: whatever is recorded goes into the current kick, so
     * submit it, then wait for the last kick actually submitted (a flush
     * with nothing recorded submits none; the kick "being recorded" would
     * then never complete). kick_id() is that next kick's id. */
    flush_pending_clear();
    if backend::flush() != 0 {
        e.error = EGL_CONTEXT_LOST;
        return NO_SYNC;
    }
    e.syncs[i].kick = backend::kick_id().wrapping_sub(1);
    e.syncs[i].used = true;
    e.sync_handle(i)
}

#[no_mangle]
pub unsafe extern "C" fn eglDestroySync(dpy: EGLDisplay, sync: EGLSync) -> EGLBoolean {
    egl_entry!();
    gl_trace!(eglDestroySync);
    let e = state();
    e.error = EGL_SUCCESS;
    if !e.valid_display(dpy) {
        return FALSE;
    }
    let Some(i) = e.lookup_sync(sync) else {
        return e.fail(EGL_BAD_PARAMETER);
    };
    e.syncs[i].used = false;
    TRUE
}

#[no_mangle]
pub unsafe extern "C" fn eglClientWaitSync(dpy: EGLDisplay, sync: EGLSync, _flags: EGLint, timeout: EGLTime) -> EGLint {
    egl_entry!();
    gl_trace!(eglClientWaitSync);
    state().error = EGL_SUCCESS;
    /* EGL_SYNC_FLUSH_COMMANDS_BIT: already flushed at creation */
    let e = state();
    if !e.valid_display(dpy) {
        return EGL_FALSE;
    }
    let Some(i) = e.lookup_sync(sync) else {
        return e.fail(EGL_BAD_PARAMETER) as EGLint;
    };
    if sync_signaled(&e.syncs[i]) {
        return EGL_CONDITION_SATISFIED;
    }
    if timeout == 0 {
        return EGL_TIMEOUT_EXPIRED;
    }
    /* EGL 1.5 3.8.1.3: the wait blocks until the sync is signaled, and
     * flushes only with EGL_SYNC_FLUSH_COMMANDS_BIT; the fence was flushed
     * at its creation. So wait for the sync's own kick and submit nothing
     * recorded since (the next frame, on a double-buffered window: DEMO13
     * submitted it into the half on screen). A backend without that wait
     * finishes, which waits for the kick too but also submits the open
     * frame. A bounded wait is not offered by the backend contract;
     * EGL_FOREVER and any finite timeout both wait for the kick. */
    let kick = e.syncs[i].kick;
    let rc = backend::wait_kick(kick).unwrap_or_else(backend::finish);
    if rc != 0 {
        return e.fail(EGL_CONTEXT_LOST) as EGLint;
    }
    EGL_CONDITION_SATISFIED
}

#[no_mangle]
pub unsafe extern "C" fn eglWaitSync(dpy: EGLDisplay, sync: EGLSync, _flags: EGLint) -> EGLBoolean {
    egl_entry!();
    gl_trace!(eglWaitSync);
    let e = state();
    e.error = EGL_SUCCESS;
    if !e.valid_display(dpy) {
        return FALSE;
    }
    /* One GPU queue: later commands already run after earlier kicks. */
    if e.lookup_sync(sync).is_some() {
        TRUE
    } else {
        e.fail(EGL_BAD_PARAMETER)
    }
}

#[no_mangle]
pub unsafe extern "C" fn eglGetSyncAttrib(
    dpy: EGLDisplay,
    sync: EGLSync,
    attribute: EGLint,
    value: *mut EGLAttrib,
) -> EGLBoolean {
    egl_entry!();
    gl_trace!(eglGetSyncAttrib);
    let e = state();
    e.error = EGL_SUCCESS;
    if !e.valid_display(dpy) {
        return FALSE;
    }
    let Some(i) = e.lookup_sync(sync).filter(|_| !value.is_null()) else {
        return e.fail(EGL_BAD_PARAMETER);
    };
    let v = match attribute {
        EGL_SYNC_TYPE => EGL_SYNC_FENCE,
        EGL_SYNC_STATUS => {
            if sync_signaled(&e.syncs[i]) {
                EGL_SIGNALED
            } else {
                EGL_UNSIGNALED
            }
        }
        EGL_SYNC_CONDITION => EGL_SYNC_PRIOR_COMMANDS_COMPLETE,
        _ => return e.fail(EGL_BAD_ATTRIBUTE),
    };
    // SAFETY: value is non-null and valid for a write (EGL contract), not
    // necessarily aligned.
    unsafe { value.write_unaligned(v as EGLAttrib) };
    TRUE
}

/* ---- images -------------------------------------------------------------- */

/// The cube-map face an image target names, 0 (+X) to 5 (-Z).
///
/// EGL 1.5 3.9 has the six EGL_GL_TEXTURE_CUBE_MAP_* targets, EGL_GL_TEXTURE_2D
/// and EGL_GL_RENDERBUFFER in the core, so an EGL that reports 1.5 answers for
/// all of them; dEQP's image.api skips the EGL_KHR_gl_texture_cubemap_image
/// check for a 1.5 display and asks.
fn cube_face(target: EGLenum) -> Option<u32> {
    let f = target as i64 - EGL_GL_TEXTURE_CUBE_MAP_POSITIVE_X as i64;
    (0..6).contains(&f).then_some(f as u32)
}

/// The storage record an image takes from a live GL texture: the whole of a
/// 2D texture, or the rows of one cube-map face. `None` when `name` is no
/// texture of that kind, or its texels are not a surface format (the
/// siblings render into and sample the storage as such).
///
/// The texture keeps using the storage, so it counts as a user; the image
/// takes over the ownership of the allocation, whole, even when what it
/// describes is one face of it.
fn image_of_texture(name: GLuint, face: Option<u32>) -> Option<EglImage> {
    let c = context::current()?;
    let t = c.texture(super::names::slot_of(c, super::names::Kind::Texture, name)?)?;
    let want = if face.is_some() { GL_TEXTURE_CUBE_MAP } else { GL_TEXTURE_2D };
    if t.mem.impl_.is_null()
        || t.width == 0
        || t.height == 0
        || t.target != want
        || t.block_bytes != 0
        || !matches!(t.texfmt, GLES_TEXFMT_RGBA8 | GLES_TEXFMT_RGB565)
    {
        return None;
    }
    let mut mem = t.mem;
    let mut alloc = OsalGpuMem::ZERO;
    if let Some(f) = face {
        /* the faces are stacked vertically in one allocation at every
         * level: face f of level 0 is f * height rows in, at the level's
         * pair-padded pitch (the arithmetic the framebuffer walks) */
        if t.layers != 6 {
            return None;
        }
        let rows = t.height as u64 * t.level_pitch(t.width) as u64 * gles_texfmt_bytes(t.texfmt) as u64;
        let off = f as u64 * rows + t.level_off[0] as u64;
        if off + rows > t.mem.size as u64 {
            return None;
        }
        alloc = t.mem;
        mem.gpu += off;
        mem.size = rows as usize;
        if !mem.cpu.is_null() {
            mem.cpu = mem.cpu.wrapping_byte_add(off as usize);
        }
        if mem.phys != 0 {
            mem.phys += off;
        }
    } else if t.layers > 1 {
        return None;
    }
    let img = EglImage {
        used: false,
        owned: !t.external, /* the image now owns pool storage */
        format: if t.bgra {
            GLES_SURF_XRGB8888
        } else if t.texfmt == GLES_TEXFMT_RGB565 {
            GLES_SURF_RGB565
        } else {
            GLES_SURF_RGBA8888
        },
        width: t.width,
        height: t.height,
        mem,
        alloc: if t.external { OsalGpuMem::ZERO } else { alloc },
        owner: None,
        /* the source texture keeps using the storage */
        refs: (!t.external) as u32,
        pe_order: t.pe_order,
    };
    t.external = true;
    Some(img)
}

/// The storage record an image takes from a live GL renderbuffer.
///
/// A colour renderbuffer holds linear RGBA8888 whatever format it was asked
/// for, which is what the image describes. A depth or stencil renderbuffer
/// owns no memory here (the depth buffer lives with the tile), so there is
/// nothing for an image to share and this answers `None`.
fn image_of_renderbuffer(name: GLuint) -> Option<EglImage> {
    let c = context::current()?;
    let rb = c.renderbuffer(super::names::slot_of(c, super::names::Kind::Renderbuffer, name)?)?;
    if rb.mem.impl_.is_null() || rb.width == 0 || rb.height == 0 {
        return None;
    }
    let img = EglImage {
        used: false,
        owned: !rb.external,
        format: GLES_SURF_RGBA8888,
        width: rb.width,
        height: rb.height,
        mem: rb.mem,
        alloc: OsalGpuMem::ZERO,
        owner: None,
        refs: (!rb.external) as u32,
        /* only the pixel engine writes a renderbuffer */
        pe_order: crate::backend::rt_rgba8888_is_bgra(),
    };
    rb.external = true;
    Some(img)
}

#[no_mangle]
pub unsafe extern "C" fn eglCreateImage(
    dpy: EGLDisplay,
    ctx: EGLContext,
    target: EGLenum,
    buffer: EGLClientBuffer,
    _attrib_list: *const EGLAttrib, /* EGL_GL_TEXTURE_LEVEL 0 and EGL_IMAGE_PRESERVED are the behaviour */
) -> EGLImage {
    egl_entry!();
    // SAFETY: the caller's contract, passed on.
    unsafe { create_image(dpy, ctx, target, buffer) }
}

/// eglCreateImage, under the lock its callers hold (eglCreateImageKHR is the
/// same call with the extension's EGLint attribute list).
///
/// # Safety
/// `buffer` names an object of `target`'s kind.
unsafe fn create_image(dpy: EGLDisplay, ctx: EGLContext, target: EGLenum, buffer: EGLClientBuffer) -> EGLImage {
    gl_trace!(eglCreateImage);
    let e = state();
    e.error = EGL_SUCCESS;
    if !e.valid_display(dpy) {
        return NO_IMAGE;
    }
    let Some(i) = e.img.iter().position(|img| !img.live()) else {
        e.error = EGL_BAD_ALLOC;
        return NO_IMAGE;
    };
    e.img[i] = EglImage::ZERO;

    if target == EGL_GL_TEXTURE_2D as EGLenum || target == EGL_GL_RENDERBUFFER as EGLenum || cube_face(target).is_some()
    {
        /* the GL object's storage, in the given context */
        let gl = e.lookup_context(ctx).map_or(core::ptr::null_mut(), |ci| e.ctx[ci].gl);
        if gl.is_null() {
            e.error = EGL_BAD_CONTEXT;
            return NO_IMAGE;
        }
        // SAFETY: a by-value read of the current-context pointer.
        let saved = unsafe { context::gl_ctx };
        // SAFETY: `gl` is a live GL context and no reference to any GL
        // context is held across the switch.
        unsafe { context::gl_context_set_current(gl) };
        /* the reference to the (temporarily current) context ends with this
         * block, before the previous context is made current again */
        let taken = {
            /* a buffer handle that does not fit a GLuint names no object */
            let name = GLuint::try_from(buffer as usize).unwrap_or(0);
            if target == EGL_GL_RENDERBUFFER as EGLenum {
                image_of_renderbuffer(name)
            } else {
                image_of_texture(name, cube_face(target))
            }
        };
        // SAFETY: as above.
        unsafe { context::gl_context_set_current(saved) };
        let Some(mut img) = taken else {
            e.error = EGL_BAD_PARAMETER;
            return NO_IMAGE;
        };
        if img.refs == 0 {
            /* the object already used an image's storage: this image is
             * one more user of that record (none: storage a C caller
             * attached, which that caller keeps owning) */
            if let Some(o) = e.storage_slot(&img.mem) {
                img.owner = Some(o as u8);
                img.alloc = OsalGpuMem::ZERO; /* the owner frees it */
                e.img[o].refs += 1;
            }
        }
        e.img[i] = img;
    } else if target == EGL_NATIVE_PIXMAP_KHR as EGLenum {
        if ctx != NO_CONTEXT {
            e.error = EGL_BAD_CONTEXT;
            return NO_IMAGE;
        }
        // SAFETY: buffer is null or a native buffer description (the
        // EGL_KHR_image_pixmap contract of this platform).
        let nb = unsafe { (buffer as *const GlesNativeWindow).as_ref() }.copied();
        let map_phys = osal::ops().and_then(|o| o.map_phys);
        let (Some((nb, format)), Some(map_phys)) = (nb.and_then(|n| Some((n, native_buffer_format(&n)?))), map_phys)
        else {
            e.error = EGL_BAD_PARAMETER;
            return NO_IMAGE;
        };
        let bytes = native_buffer_bytes(&nb);
        // SAFETY: the image's descriptor is a valid exclusive reference for the call.
        if unsafe { map_phys(backend::heap_base(GLES_HEAP_DATA), nb.phys, bytes, 0, &mut e.img[i].mem) } != 0 {
            e.error = EGL_BAD_ALLOC;
            return NO_IMAGE;
        }
        e.img[i].format = format;
        e.img[i].width = nb.width;
        e.img[i].height = nb.height;
    } else {
        e.error = EGL_BAD_PARAMETER;
        return NO_IMAGE;
    }
    e.img[i].used = true;
    e.image_handle(i)
}

/// eglDestroyImage (traced here: eglTerminate enters it per image).
fn destroy_image(e: &mut EglState, dpy: EGLDisplay, image: EGLImage) -> EGLBoolean {
    gl_trace!(eglDestroyImage);
    if !e.valid_display(dpy) {
        return FALSE;
    }
    let Some(i) = e.lookup_image(image) else {
        return e.fail(EGL_BAD_PARAMETER);
    };
    /* the handle goes; the storage stays while a sibling texture or
     * renderbuffer (or another image) still uses it (EGL_KHR_image_base:
     * the siblings keep the image's content) */
    e.img[i].used = false;
    if !e.img[i].live() {
        e.storage_release(i);
    }
    TRUE
}

/// A GL object stopped using `mem`, storage an EGLImage owns (see
/// [`EglImage`]). Storage that belongs to no image (attached by a C caller
/// through gl_texture_attach_external) is the caller's: nothing to do.
fn image_storage_release_in(e: &mut EglState, mem: &OsalGpuMem) {
    if let Some(i) = e.storage_slot(mem) {
        e.storage_unref(i);
    }
}

/// [`image_storage_release_in`] from a GL entry point (no EGL state held).
///
/// The lock goes here rather than at the GL entry point: this is reached
/// from glDeleteTextures and from a context being destroyed, neither of
/// which holds it. The `_in` form is the one EGL's own paths take, and they
/// hold it already.
pub(crate) fn image_storage_release(mem: &OsalGpuMem) {
    let _lock = osal::Lock::take();
    image_storage_release_in(state(), mem);
}

/// A GL object started using `mem` (a glEGLImageTarget* sibling).
/// A texture on an EGLImage's storage was converted to the pixel engine's
/// channel order (`texture::take_pe_order`): the image records it, so a
/// texture attached to it later (glEGLImageTargetTexture2DOES) knows.
pub(crate) fn image_note_pe_order(gpu: u64) {
    for img in state().img.iter_mut() {
        if img.live() && img.mem.gpu == gpu {
            img.pe_order = true;
        }
    }
}

fn image_storage_acquire_in(e: &mut EglState, mem: &OsalGpuMem) {
    if let Some(i) = e.storage_slot(mem) {
        e.img[i].refs += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn eglDestroyImage(dpy: EGLDisplay, image: EGLImage) -> EGLBoolean {
    egl_entry!();
    state().error = EGL_SUCCESS;
    destroy_image(state(), dpy, image)
}

/* ---- EGL_KHR_image_base ---------------------------------------------------- */

/// eglCreateImageKHR: eglCreateImage with the extension's EGLint attribute
/// list in place of EGL 1.5's EGLAttrib one. Both lists say the same thing
/// here (EGL_GL_TEXTURE_LEVEL 0 and EGL_IMAGE_PRESERVED are the behaviour),
/// and the KHR target tokens are the core ones, so the core call serves.
/// EGL_KHR_image_base is advertised, so eglGetProcAddress has to answer for
/// its two entry points -- dEQP asks for every function of every extension
/// named in EGL_EXTENSIONS.
///
/// # Safety
/// As eglCreateImage: `buffer` names an object of `target`'s kind.
#[no_mangle]
pub unsafe extern "C" fn eglCreateImageKHR(
    dpy: EGLDisplay,
    ctx: EGLContext,
    target: EGLenum,
    buffer: EGLClientBuffer,
    _attrib_list: *const EGLint,
) -> EGLImage {
    egl_entry!();
    // SAFETY: the caller's contract, passed on; the list is not read.
    unsafe { create_image(dpy, ctx, target, buffer) }
}

/// eglDestroyImageKHR: eglDestroyImage under the extension's name.
///
/// # Safety
/// As eglDestroyImage.
#[no_mangle]
pub unsafe extern "C" fn eglDestroyImageKHR(dpy: EGLDisplay, image: EGLImage) -> EGLBoolean {
    egl_entry!();
    state().error = EGL_SUCCESS;
    destroy_image(state(), dpy, image)
}

/* ---- GL_OES_EGL_image ------------------------------------------------------ */

#[no_mangle]
pub unsafe extern "C" fn glEGLImageTargetTexture2DOES(target: GLenum, image: GLeglImageOES) {
    /* a GL entry point, but the image table it reads is EGL's: a second
     * thread is refused before it reads it (context::current would refuse
     * only after) */
    if context::foreign_thread() {
        return;
    }
    let _lock = osal::Lock::take();
    gl_trace!(glEGLImageTargetTexture2DOES);
    const FUNC: &str = "glEGLImageTargetTexture2DOES";
    let e = state();
    let img = e.lookup_image(image).map(|i| e.img[i]);

    let Some(c) = context::current() else {
        return;
    };
    if target != GL_TEXTURE_2D {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    let Some(img) = img else {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    };
    /* the attach releases what the texture held through the EGL state
     * (`e` is not used again; the state is fetched anew below) */
    if texture::texture_attach_external(c, &img.mem, img.width, img.height, img.format) != 0 {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    texture::set_bound_2d_pe_order(c, img.pe_order && img.format == GLES_SURF_RGBA8888);
    image_storage_acquire_in(state(), &img.mem);
}

#[no_mangle]
pub unsafe extern "C" fn glEGLImageTargetRenderbufferStorageOES(target: GLenum, image: GLeglImageOES) {
    /* a GL entry point, but the image table it reads is EGL's: a second
     * thread is refused before it reads it (context::current would refuse
     * only after) */
    if context::foreign_thread() {
        return;
    }
    let _lock = osal::Lock::take();
    gl_trace!(glEGLImageTargetRenderbufferStorageOES);
    const FUNC: &str = "glEGLImageTargetRenderbufferStorageOES";
    let e = state();
    let img = e.lookup_image(image).map(|i| e.img[i]);

    let Some(c) = context::current() else {
        return;
    };
    if target != GL_RENDERBUFFER {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    /* RGBA render targets only */
    let Some(img) = img.filter(|img| img.format == GLES_SURF_RGBA8888) else {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    };
    if framebuffer::renderbuffer_attach_external(c, &img.mem, img.width, img.height) != 0 {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    image_storage_acquire_in(state(), &img.mem);
}

/* ---- eglGetProcAddress --------------------------------------------------- */

type ProcFn = unsafe extern "C" fn();

struct Proc {
    /// NUL-terminated.
    name: &'static [u8],
    f: ProcFn,
}

/// The GL entry points of the other modules, by their C symbols: the table
/// only needs their addresses, so the signatures are not repeated here.
mod sym {
    extern "C" {
        pub fn glActiveTexture();
        pub fn glGetStringi();
        pub fn glVertexAttribDivisor();
        pub fn glVertexAttribIPointer();
        pub fn glDrawArraysInstanced();
        pub fn glDrawElementsInstanced();
        pub fn glDrawRangeElements();
        pub fn glMapBufferRange();
        pub fn glUnmapBuffer();
        /* GL_OES_mapbuffer (gl/buffer.rs) */
        pub fn glMapBufferOES();
        pub fn glUnmapBufferOES();
        pub fn glGetBufferPointervOES();
        pub fn glFlushMappedBufferRange();
        pub fn glInvalidateFramebuffer();
        pub fn glInvalidateSubFramebuffer();
        pub fn glDrawBuffers();
        pub fn glReadBuffer();
        pub fn glBlitFramebuffer();
        pub fn glRenderbufferStorageMultisample();
        pub fn glGetInternalformativ();
        pub fn glAttachShader();
        pub fn glBindAttribLocation();
        pub fn glBindBuffer();
        pub fn glBindFramebuffer();
        pub fn glBindRenderbuffer();
        pub fn glBindTexture();
        pub fn glBlendColor();
        pub fn glBlendEquation();
        pub fn glBlendEquationSeparate();
        pub fn glBlendFunc();
        pub fn glBlendFuncSeparate();
        pub fn glBufferData();
        pub fn glBufferSubData();
        pub fn glCheckFramebufferStatus();
        pub fn glClear();
        pub fn glClearColor();
        pub fn glClearDepthf();
        pub fn glClearStencil();
        pub fn glColorMask();
        pub fn glCompileShader();
        pub fn glCompressedTexImage2D();
        pub fn glCompressedTexSubImage2D();
        pub fn glCopyTexImage2D();
        pub fn glCopyTexSubImage2D();
        pub fn glCreateProgram();
        pub fn glCreateShader();
        pub fn glCullFace();
        pub fn glDeleteBuffers();
        pub fn glDeleteFramebuffers();
        pub fn glDeleteProgram();
        pub fn glDeleteRenderbuffers();
        pub fn glDeleteShader();
        pub fn glDeleteTextures();
        pub fn glDepthFunc();
        pub fn glDepthMask();
        pub fn glDepthRangef();
        pub fn glDetachShader();
        pub fn glDisable();
        pub fn glDisableVertexAttribArray();
        pub fn glDrawArrays();
        pub fn glDrawElements();
        pub fn glEnable();
        pub fn glEnableVertexAttribArray();
        pub fn glFinish();
        pub fn glFlush();
        pub fn glFramebufferRenderbuffer();
        pub fn glFramebufferTexture2D();
        pub fn glFrontFace();
        pub fn glGenBuffers();
        pub fn glGenVertexArraysOES();
        pub fn glBindVertexArrayOES();
        pub fn glDeleteVertexArraysOES();
        pub fn glIsVertexArrayOES();
        /* GL_EXT_debug_marker (gl/state.rs) */
        pub fn glInsertEventMarkerEXT();
        pub fn glPushGroupMarkerEXT();
        pub fn glPopGroupMarkerEXT();
        /* the same calls under their ES 3.0 core names (gl/vao.rs) */
        pub fn glGenVertexArrays();
        pub fn glBindVertexArray();
        pub fn glDeleteVertexArrays();
        pub fn glIsVertexArray();
        pub fn glGenFramebuffers();
        pub fn glGenRenderbuffers();
        pub fn glGenTextures();
        pub fn glGenerateMipmap();
        pub fn glGetActiveAttrib();
        pub fn glGetActiveUniform();
        pub fn glGetAttachedShaders();
        pub fn glGetAttribLocation();
        pub fn glGetBooleanv();
        pub fn glGetBufferParameteriv();
        pub fn glGetError();
        pub fn glGetFloatv();
        pub fn glGetFramebufferAttachmentParameteriv();
        pub fn glGetIntegerv();
        pub fn glGetProgramInfoLog();
        pub fn glGetProgramiv();
        pub fn glGetRenderbufferParameteriv();
        pub fn glGetShaderInfoLog();
        pub fn glGetShaderPrecisionFormat();
        pub fn glGetShaderSource();
        pub fn glGetShaderiv();
        pub fn glGetString();
        pub fn glGetTexParameterfv();
        pub fn glGetTexParameteriv();
        pub fn glGetUniformLocation();
        pub fn glGetUniformfv();
        pub fn glGetUniformiv();
        pub fn glGetVertexAttribPointerv();
        pub fn glGetVertexAttribfv();
        pub fn glGetVertexAttribiv();
        pub fn glHint();
        pub fn glIsBuffer();
        pub fn glIsEnabled();
        pub fn glIsFramebuffer();
        pub fn glIsProgram();
        pub fn glIsRenderbuffer();
        pub fn glIsShader();
        pub fn glIsTexture();
        pub fn glLineWidth();
        pub fn glLinkProgram();
        pub fn glPixelStorei();
        pub fn glPolygonOffset();
        pub fn glReadPixels();
        pub fn glReleaseShaderCompiler();
        pub fn glRenderbufferStorage();
        pub fn glSampleCoverage();
        pub fn glScissor();
        pub fn glShaderBinary();
        pub fn glShaderSource();
        pub fn glStencilFunc();
        pub fn glStencilFuncSeparate();
        pub fn glStencilMask();
        pub fn glStencilMaskSeparate();
        pub fn glStencilOp();
        pub fn glStencilOpSeparate();
        pub fn glTexImage2D();
        pub fn glTexParameterf();
        pub fn glTexParameterfv();
        pub fn glTexParameteri();
        pub fn glTexParameteriv();
        pub fn glTexSubImage2D();
        pub fn glUniform1f();
        pub fn glUniform1fv();
        pub fn glUniform1i();
        pub fn glUniform1iv();
        pub fn glUniform2f();
        pub fn glUniform2fv();
        pub fn glUniform2i();
        pub fn glUniform2iv();
        pub fn glUniform3f();
        pub fn glUniform3fv();
        pub fn glUniform3i();
        pub fn glUniform3iv();
        pub fn glUniform4f();
        pub fn glUniform4fv();
        pub fn glUniform4i();
        pub fn glUniform4iv();
        pub fn glUniform1ui();
        pub fn glUniform2ui();
        pub fn glUniform3ui();
        pub fn glUniform4ui();
        pub fn glUniform1uiv();
        pub fn glUniform2uiv();
        pub fn glUniform3uiv();
        pub fn glUniform4uiv();
        pub fn glTexImage3D();
        pub fn glTexSubImage3D();
        pub fn glCompressedTexImage3D();
        pub fn glTexStorage2D();
        pub fn glTexStorage3D();
        pub fn glFramebufferTextureLayer();
        pub fn glUniformMatrix2fv();
        pub fn glUniformMatrix3fv();
        pub fn glUniformMatrix4fv();
        pub fn glUseProgram();
        pub fn glValidateProgram();
        pub fn glVertexAttrib1f();
        pub fn glVertexAttrib1fv();
        pub fn glVertexAttrib2f();
        pub fn glVertexAttrib2fv();
        pub fn glVertexAttrib3f();
        pub fn glVertexAttrib3fv();
        pub fn glVertexAttrib4f();
        pub fn glVertexAttrib4fv();
        pub fn glVertexAttribPointer();
        pub fn glViewport();
        pub fn glBeginQuery();
        pub fn glBeginTransformFeedback();
        pub fn glBindBufferBase();
        pub fn glBindBufferRange();
        pub fn glBindSampler();
        pub fn glBindTransformFeedback();
        pub fn glClearBufferfi();
        pub fn glClearBufferfv();
        pub fn glClearBufferiv();
        pub fn glClearBufferuiv();
        pub fn glClientWaitSync();
        pub fn glCompressedTexSubImage3D();
        pub fn glCopyBufferSubData();
        pub fn glCopyTexSubImage3D();
        pub fn glDeleteQueries();
        pub fn glDeleteSamplers();
        pub fn glDeleteSync();
        pub fn glDeleteTransformFeedbacks();
        pub fn glEndQuery();
        pub fn glEndTransformFeedback();
        pub fn glFenceSync();
        pub fn glGenQueries();
        pub fn glGenSamplers();
        pub fn glGenTransformFeedbacks();
        pub fn glGetActiveUniformBlockName();
        pub fn glGetActiveUniformBlockiv();
        pub fn glGetActiveUniformsiv();
        pub fn glGetBufferParameteri64v();
        pub fn glGetBufferPointerv();
        pub fn glGetFragDataLocation();
        pub fn glGetInteger64i_v();
        pub fn glGetInteger64v();
        pub fn glGetIntegeri_v();
        pub fn glGetProgramBinary();
        pub fn glGetQueryObjectuiv();
        pub fn glGetQueryiv();
        pub fn glGetSamplerParameterfv();
        pub fn glGetSamplerParameteriv();
        pub fn glGetSynciv();
        pub fn glGetTransformFeedbackVarying();
        pub fn glGetUniformBlockIndex();
        pub fn glGetUniformIndices();
        pub fn glGetUniformuiv();
        pub fn glGetVertexAttribIiv();
        pub fn glGetVertexAttribIuiv();
        pub fn glIsQuery();
        pub fn glIsSampler();
        pub fn glIsSync();
        pub fn glIsTransformFeedback();
        pub fn glPauseTransformFeedback();
        pub fn glProgramBinary();
        pub fn glProgramParameteri();
        pub fn glResumeTransformFeedback();
        pub fn glSamplerParameterf();
        pub fn glSamplerParameterfv();
        pub fn glSamplerParameteri();
        pub fn glSamplerParameteriv();
        pub fn glTransformFeedbackVaryings();
        pub fn glUniformBlockBinding();
        pub fn glUniformMatrix2x3fv();
        pub fn glUniformMatrix2x4fv();
        pub fn glUniformMatrix3x2fv();
        pub fn glUniformMatrix3x4fv();
        pub fn glUniformMatrix4x2fv();
        pub fn glUniformMatrix4x3fv();
        pub fn glVertexAttribI4i();
        pub fn glVertexAttribI4iv();
        pub fn glVertexAttribI4ui();
        pub fn glVertexAttribI4uiv();
        pub fn glWaitSync();
    }
}

/// An entry point of another module.
macro_rules! gl {
    ($name:ident) => {
        Proc { name: concat!(stringify!($name), "\0").as_bytes(), f: sym::$name }
    };
    ($name:literal, $f:ident) => {
        Proc { name: concat!($name, "\0").as_bytes(), f: sym::$f }
    };
}

/// An entry point of this file.
macro_rules! own {
    ($name:ident) => {
        Proc {
            name: concat!(stringify!($name), "\0").as_bytes(),
            // SAFETY: a function address as `void (*)(void)`, which the
            // caller casts back to the real type before calling (the
            // __eglMustCastToProperFunctionPointerType contract).
            f: unsafe { core::mem::transmute::<*const (), ProcFn>($name as *const ()) },
        }
    };
}

static PROCS: &[Proc] = &[
    gl!(glActiveTexture),
    /* OpenGL ES 3.0 entry points (the core's subset; see docs) */
    gl!(glGetStringi),
    gl!(glVertexAttribDivisor),
    gl!(glVertexAttribIPointer),
    gl!(glDrawArraysInstanced),
    gl!(glDrawElementsInstanced),
    gl!(glDrawRangeElements),
    gl!(glMapBufferRange),
    gl!(glUnmapBuffer),
    gl!(glMapBufferOES),
    gl!(glUnmapBufferOES),
    gl!(glGetBufferPointervOES),
    gl!(glFlushMappedBufferRange),
    gl!(glInvalidateFramebuffer),
    gl!(glInvalidateSubFramebuffer),
    gl!(glDrawBuffers),
    gl!(glReadBuffer),
    gl!(glBlitFramebuffer),
    gl!(glRenderbufferStorageMultisample),
    gl!(glGetInternalformativ),
    /* The rest of OpenGL ES 3.0. EGL 1.5 3.10 requires eglGetProcAddress to
     * answer for every EGL and client-API function, not only the ones an
     * extension added: a conformance run resolves the whole API through it
     * and calls what it gets back, so a name missing here is a jump to
     * address zero rather than a reported failure. */
    gl!(glBeginQuery),
    gl!(glBeginTransformFeedback),
    gl!(glBindBufferBase),
    gl!(glBindBufferRange),
    gl!(glBindSampler),
    gl!(glBindTransformFeedback),
    gl!(glClearBufferfi),
    gl!(glClearBufferfv),
    gl!(glClearBufferiv),
    gl!(glClearBufferuiv),
    gl!(glClientWaitSync),
    gl!(glCompressedTexSubImage3D),
    gl!(glCopyBufferSubData),
    gl!(glCopyTexSubImage3D),
    gl!(glDeleteQueries),
    gl!(glDeleteSamplers),
    gl!(glDeleteSync),
    gl!(glDeleteTransformFeedbacks),
    gl!(glEndQuery),
    gl!(glEndTransformFeedback),
    gl!(glFenceSync),
    gl!(glGenQueries),
    gl!(glGenSamplers),
    gl!(glGenTransformFeedbacks),
    gl!(glGetActiveUniformBlockName),
    gl!(glGetActiveUniformBlockiv),
    gl!(glGetActiveUniformsiv),
    gl!(glGetBufferParameteri64v),
    gl!(glGetBufferPointerv),
    gl!(glGetFragDataLocation),
    gl!(glGetInteger64i_v),
    gl!(glGetInteger64v),
    gl!(glGetIntegeri_v),
    gl!(glGetProgramBinary),
    gl!(glGetQueryObjectuiv),
    gl!(glGetQueryiv),
    gl!(glGetSamplerParameterfv),
    gl!(glGetSamplerParameteriv),
    gl!(glGetSynciv),
    gl!(glGetTransformFeedbackVarying),
    gl!(glGetUniformBlockIndex),
    gl!(glGetUniformIndices),
    gl!(glGetUniformuiv),
    gl!(glGetVertexAttribIiv),
    gl!(glGetVertexAttribIuiv),
    gl!(glIsQuery),
    gl!(glIsSampler),
    gl!(glIsSync),
    gl!(glIsTransformFeedback),
    gl!(glPauseTransformFeedback),
    gl!(glProgramBinary),
    gl!(glProgramParameteri),
    gl!(glResumeTransformFeedback),
    gl!(glSamplerParameterf),
    gl!(glSamplerParameterfv),
    gl!(glSamplerParameteri),
    gl!(glSamplerParameteriv),
    gl!(glTransformFeedbackVaryings),
    gl!(glUniformBlockBinding),
    gl!(glUniformMatrix2x3fv),
    gl!(glUniformMatrix2x4fv),
    gl!(glUniformMatrix3x2fv),
    gl!(glUniformMatrix3x4fv),
    gl!(glUniformMatrix4x2fv),
    gl!(glUniformMatrix4x3fv),
    gl!(glVertexAttribI4i),
    gl!(glVertexAttribI4iv),
    gl!(glVertexAttribI4ui),
    gl!(glVertexAttribI4uiv),
    gl!(glWaitSync),
    gl!(glAttachShader),
    gl!(glBindAttribLocation),
    gl!(glBindBuffer),
    gl!(glBindFramebuffer),
    gl!(glBindRenderbuffer),
    gl!(glBindTexture),
    gl!(glBlendColor),
    gl!(glBlendEquation),
    gl!(glBlendEquationSeparate),
    gl!(glBlendFunc),
    gl!(glBlendFuncSeparate),
    gl!(glBufferData),
    gl!(glBufferSubData),
    gl!(glCheckFramebufferStatus),
    gl!(glClear),
    gl!(glClearColor),
    gl!(glClearDepthf),
    gl!(glClearStencil),
    gl!(glColorMask),
    gl!(glCompileShader),
    gl!(glCompressedTexImage2D),
    gl!(glCompressedTexSubImage2D),
    gl!(glCopyTexImage2D),
    gl!(glCopyTexSubImage2D),
    gl!(glCreateProgram),
    gl!(glCreateShader),
    gl!(glCullFace),
    gl!(glDeleteBuffers),
    gl!(glDeleteFramebuffers),
    gl!(glDeleteProgram),
    gl!(glDeleteRenderbuffers),
    gl!(glDeleteShader),
    gl!(glDeleteTextures),
    gl!(glDepthFunc),
    gl!(glDepthMask),
    gl!(glDepthRangef),
    gl!(glDetachShader),
    gl!(glDisable),
    gl!(glDisableVertexAttribArray),
    gl!(glDrawArrays),
    gl!(glDrawElements),
    gl!(glEnable),
    gl!(glEnableVertexAttribArray),
    gl!(glFinish),
    gl!(glFlush),
    gl!(glFramebufferRenderbuffer),
    gl!(glFramebufferTexture2D),
    gl!(glFrontFace),
    gl!(glGenBuffers),
    gl!(glGenVertexArraysOES),
    gl!(glBindVertexArrayOES),
    gl!(glDeleteVertexArraysOES),
    gl!(glIsVertexArrayOES),
    gl!(glInsertEventMarkerEXT),
    gl!(glPushGroupMarkerEXT),
    gl!(glPopGroupMarkerEXT),
    /* ES 3.0 core spellings resolve to the same implementation (glow tries these first) */
    gl!(glGenVertexArrays),
    gl!(glBindVertexArray),
    gl!(glDeleteVertexArrays),
    gl!(glIsVertexArray),
    gl!(glGenFramebuffers),
    gl!(glGenRenderbuffers),
    gl!(glGenTextures),
    gl!(glGenerateMipmap),
    gl!(glGetActiveAttrib),
    gl!(glGetActiveUniform),
    gl!(glGetAttachedShaders),
    gl!(glGetAttribLocation),
    gl!(glGetBooleanv),
    gl!(glGetBufferParameteriv),
    gl!(glGetError),
    gl!(glGetFloatv),
    gl!(glGetFramebufferAttachmentParameteriv),
    gl!(glGetIntegerv),
    gl!(glGetProgramInfoLog),
    gl!(glGetProgramiv),
    gl!(glGetRenderbufferParameteriv),
    gl!(glGetShaderInfoLog),
    gl!(glGetShaderPrecisionFormat),
    gl!(glGetShaderSource),
    gl!(glGetShaderiv),
    gl!(glGetString),
    gl!(glGetTexParameterfv),
    gl!(glGetTexParameteriv),
    gl!(glGetUniformLocation),
    gl!(glGetUniformfv),
    gl!(glGetUniformiv),
    gl!(glGetVertexAttribPointerv),
    gl!(glGetVertexAttribfv),
    gl!(glGetVertexAttribiv),
    gl!(glHint),
    gl!(glIsBuffer),
    gl!(glIsEnabled),
    gl!(glIsFramebuffer),
    gl!(glIsProgram),
    gl!(glIsRenderbuffer),
    gl!(glIsShader),
    gl!(glIsTexture),
    gl!(glLineWidth),
    gl!(glLinkProgram),
    gl!(glPixelStorei),
    gl!(glPolygonOffset),
    gl!(glReadPixels),
    gl!(glReleaseShaderCompiler),
    gl!(glRenderbufferStorage),
    gl!(glSampleCoverage),
    gl!(glScissor),
    gl!(glShaderBinary),
    gl!(glShaderSource),
    gl!(glStencilFunc),
    gl!(glStencilFuncSeparate),
    gl!(glStencilMask),
    gl!(glStencilMaskSeparate),
    gl!(glStencilOp),
    gl!(glStencilOpSeparate),
    gl!(glTexImage2D),
    gl!(glTexParameterf),
    gl!(glTexParameterfv),
    gl!(glTexParameteri),
    gl!(glTexParameteriv),
    gl!(glTexSubImage2D),
    gl!(glUniform1f),
    gl!(glUniform1fv),
    gl!(glUniform1i),
    gl!(glUniform1iv),
    gl!(glUniform2f),
    gl!(glUniform2fv),
    gl!(glUniform2i),
    gl!(glUniform2iv),
    gl!(glUniform3f),
    gl!(glUniform3fv),
    gl!(glUniform3i),
    gl!(glUniform3iv),
    gl!(glUniform4f),
    gl!(glUniform4fv),
    gl!(glUniform4i),
    gl!(glUniform4iv),
    gl!(glUniform1ui),
    gl!(glUniform2ui),
    gl!(glUniform3ui),
    gl!(glUniform4ui),
    gl!(glUniform1uiv),
    gl!(glUniform2uiv),
    gl!(glUniform3uiv),
    gl!(glUniform4uiv),
    gl!(glTexImage3D),
    gl!(glTexSubImage3D),
    gl!(glCompressedTexImage3D),
    gl!(glTexStorage2D),
    gl!(glTexStorage3D),
    gl!(glFramebufferTextureLayer),
    gl!(glUniformMatrix2fv),
    gl!(glUniformMatrix3fv),
    gl!(glUniformMatrix4fv),
    gl!(glUseProgram),
    gl!(glValidateProgram),
    gl!(glVertexAttrib1f),
    gl!(glVertexAttrib1fv),
    gl!(glVertexAttrib2f),
    gl!(glVertexAttrib2fv),
    gl!(glVertexAttrib3f),
    gl!(glVertexAttrib3fv),
    gl!(glVertexAttrib4f),
    gl!(glVertexAttrib4fv),
    gl!(glVertexAttribPointer),
    gl!(glViewport),
    own!(glEGLImageTargetTexture2DOES),
    own!(glEGLImageTargetRenderbufferStorageOES),
    own!(eglBindAPI),
    own!(eglBindTexImage),
    own!(eglChooseConfig),
    own!(eglClientWaitSync),
    own!(eglCopyBuffers),
    own!(eglCreateContext),
    own!(eglCreateImage),
    own!(eglCreateImageKHR),
    own!(eglCreatePbufferFromClientBuffer),
    own!(eglCreatePbufferSurface),
    own!(eglCreatePixmapSurface),
    own!(eglCreatePlatformPixmapSurface),
    own!(eglCreatePlatformWindowSurface),
    own!(eglCreateSync),
    own!(eglCreateWindowSurface),
    own!(eglDestroyContext),
    own!(eglDestroyImage),
    own!(eglDestroyImageKHR),
    own!(eglDestroySurface),
    own!(eglDestroySync),
    own!(eglGetConfigAttrib),
    own!(eglGetConfigs),
    own!(eglGetCurrentContext),
    own!(eglGetCurrentDisplay),
    own!(eglGetCurrentSurface),
    own!(eglGetDisplay),
    own!(eglGetError),
    own!(eglGetPlatformDisplay),
    own!(eglGetProcAddress),
    own!(eglGetSyncAttrib),
    own!(eglInitialize),
    own!(eglMakeCurrent),
    own!(eglQueryAPI),
    own!(eglQueryContext),
    own!(eglQueryString),
    own!(eglQuerySurface),
    own!(eglReleaseTexImage),
    own!(eglReleaseThread),
    own!(eglSurfaceAttrib),
    own!(eglSwapBuffers),
    own!(eglSwapInterval),
    own!(eglTerminate),
    own!(eglWaitClient),
    own!(eglWaitGL),
    own!(eglWaitNative),
    own!(eglWaitSync),
];

#[no_mangle]
pub unsafe extern "C" fn eglGetProcAddress(procname: *const c_char) -> EglProc {
    egl_entry!();
    gl_trace!(eglGetProcAddress);
    state().error = EGL_SUCCESS;
    if procname.is_null() {
        return None;
    }
    // SAFETY: procname is a NUL-terminated string (EGL contract).
    let name = unsafe { crate::util::cstr_bytes(procname) };
    PROCS.iter().find(|p| &p.name[..p.name.len() - 1] == name).map(|p| p.f)
}

/// Unit tests: the EGL state as the process starts (testutil::TestGl calls
/// this when it is created and dropped, so a test that panicked with EGL
/// initialised leaves nothing behind).
///
/// **Call it only while holding the test guard** -- that is, let `TestGl` do
/// it. It throws away the display, the surface table and the current
/// bindings, which belong to whichever test is running; `TestGl` serialises
/// those, so a call from outside one lands in the middle of another test's
/// EGL objects. A test that did exactly that cost about one run in a hundred
/// for a while, failing somewhere else entirely with an EGL call returning
/// EGL_FALSE.
#[cfg(test)]
pub(crate) fn reset_for_tests() {
    // SAFETY: tests holding a TestGl are serialised; no EGL call is running.
    unsafe {
        *E.get() = EglState {
            inited: false,
            platform_arg: core::ptr::null_mut(),
            error: EGL_SUCCESS,
            api: 0,
            ctx: [EglContext::ZERO; MAX_CONTEXTS],
            surf: [EglSurface::ZERO; EGL_MAX_SURFACES],
            img: [EglImage::ZERO; EGL_MAX_IMAGES],
            syncs: [EglSyncObj { used: false, kick: 0 }; EGL_MAX_SYNCS],
            cur_ctx: None,
            cur_draw: None,
            swap_interval: 1,
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gl::consts::{
        GL_COLOR_ATTACHMENT0, GL_COLOR_BUFFER_BIT, GL_COMPRESSED_RGB8_ETC2, GL_DEPTH_COMPONENT16, GL_FRAMEBUFFER,
        GL_FRAMEBUFFER_COMPLETE, GL_NO_ERROR, GL_RGBA, GL_RGBA16F, GL_RGBA4, GL_RGBA8, GL_TEXTURE_CUBE_MAP,
        GL_TEXTURE_CUBE_MAP_POSITIVE_X, GL_UNSIGNED_BYTE,
    };
    use crate::gl::framebuffer::{
        glBindFramebuffer, glBindRenderbuffer, glCheckFramebufferStatus, glDeleteRenderbuffers,
        glFramebufferRenderbuffer, glGenFramebuffers, glGenRenderbuffers, glReadPixels, glRenderbufferStorage,
    };
    use crate::gl::state::{glClear, glClearColor, glFinish, glGetError};
    use crate::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glIsTexture, glTexImage2D};
    use crate::osal::OsalOps;
    use crate::testutil::{CacheOp, MemEvent, TestGl, HEAP_DATA};
    use std::boxed::Box;
    use std::ffi::CStr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::vec::Vec;

    /* ---- the EGL guard ----------------------------------------------------------
     * EGL keeps its own global state, which TestGl knows nothing about: this
     * guard starts every test from a clean, terminated EGL and leaves it that
     * way (also when the test panics). Declare it AFTER the TestGl so it is
     * dropped first, while the mocks are still installed. */

    fn reset_state() {
        let e = state();
        e.syncs = [EglSyncObj { used: false, kick: 0 }; EGL_MAX_SYNCS]; /* eglTerminate keeps them */
        e.platform_arg = core::ptr::null_mut();
        e.error = EGL_SUCCESS;
        e.api = 0;
    }

    struct Egl<'a> {
        gl: &'a TestGl,
        dpy: EGLDisplay,
        /// The TestGl context was destroyed to make room in the 2-slot GL pool.
        freed_testgl: bool,
    }

    impl<'a> Egl<'a> {
        /// The display, not initialised.
        fn uninit(gl: &'a TestGl) -> Self {
            assert!(!state().inited, "a previous test left EGL initialised");
            reset_state();
            // SAFETY: plain EGL call.
            let dpy = unsafe { eglGetDisplay(core::ptr::null_mut()) };
            Egl { gl, dpy, freed_testgl: false }
        }

        /// The display, initialised.
        fn new(gl: &'a TestGl) -> Self {
            let egl = Egl::uninit(gl);
            // SAFETY: null major/minor are allowed.
            assert_eq!(unsafe { eglInitialize(egl.dpy, core::ptr::null_mut(), core::ptr::null_mut()) }, TRUE);
            gl.clear_recordings();
            egl
        }

        /// Give the GL pool slot TestGl holds to EGL (both EGL contexts can then
        /// be made current). Drop puts a context back for TestGl::drop.
        fn free_testgl_context(&mut self) {
            // SAFETY: the context is live and no reference to it is held.
            unsafe { context::gl_context_destroy(self.gl.ctx()) };
            self.freed_testgl = true;
            self.gl.clear_recordings();
        }

        fn pbuffer(&self, config: EGLConfig, w: EGLint, h: EGLint) -> EGLSurface {
            let attrs = [EGL_WIDTH, w, EGL_HEIGHT, h, EGL_NONE];
            // SAFETY: an EGL_NONE-terminated list.
            unsafe { eglCreatePbufferSurface(self.dpy, config, attrs.as_ptr()) }
        }

        fn context(&self, config: EGLConfig) -> EGLContext {
            self.shared_context(config, NO_CONTEXT)
        }

        /// A context sharing `share`'s objects (NO_CONTEXT: its own).
        fn shared_context(&self, config: EGLConfig, share: EGLContext) -> EGLContext {
            let attrs = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
            // SAFETY: an EGL_NONE-terminated list.
            unsafe { eglCreateContext(self.dpy, config, share, attrs.as_ptr()) }
        }

        fn make_current(&self, surface: EGLSurface, ctx: EGLContext) -> EGLBoolean {
            // SAFETY: plain EGL call.
            unsafe { eglMakeCurrent(self.dpy, surface, surface, ctx) }
        }

        fn release(&self) -> EGLBoolean {
            self.make_current(NO_SURFACE, NO_CONTEXT)
        }

        /// A w x h pbuffer and a context, current; recordings cleared.
        fn current(&self, w: EGLint, h: EGLint) -> (EGLSurface, EGLContext) {
            let s = self.pbuffer(CONFIG_1X, w, h);
            let c = self.context(CONFIG_1X);
            assert!(!s.is_null() && !c.is_null());
            assert_eq!(self.make_current(s, c), TRUE);
            self.gl.clear_recordings();
            (s, c)
        }
    }

    /// Step around a known bug (destroying_the_context_leaves_the_images_storage_alone):
    /// destroying a GL context frees the storage its EGLImage-backed textures
    /// and renderbuffers do not own, and the images free it again. The mock
    /// reports that double free, which would fail every test that leaves an
    /// image target behind when EGL is terminated.
    impl Drop for Egl<'_> {
        fn drop(&mut self) {
            // SAFETY: plain EGL / context calls; no reference to a GL context is held.
            unsafe {
                eglMakeCurrent(self.dpy, NO_SURFACE, NO_SURFACE, NO_CONTEXT);
                eglTerminate(self.dpy);
                reset_state();
                if self.freed_testgl {
                    /* every slot is free again: the first one is TestGl's */
                    context::gl_context_create(8, 8);
                }
                context::gl_context_set_current(self.gl.ctx());
            }
        }
    }

    fn egl_error() -> EGLint {
        // SAFETY: plain EGL call.
        unsafe { eglGetError() }
    }

    fn gl_error() -> GLenum {
        // SAFETY: plain GL call.
        unsafe { glGetError() }
    }

    /// The pool entry behind a surface handle.
    fn surf(s: EGLSurface) -> EglSurface {
        let e = state();
        let i = e.lookup_surface(s).expect("a live surface");
        e.surf[i]
    }

    /// The pool entry behind an image handle.
    fn image(img: EGLImage) -> EglImage {
        let e = state();
        let i = e.lookup_image(img).expect("a live image");
        e.img[i]
    }

    /// eglChooseConfig: the matching config ids in order, or the EGL error.
    fn choose(dpy: EGLDisplay, attrs: &[EGLint]) -> Result<Vec<usize>, EGLint> {
        assert_eq!(attrs.last(), Some(&EGL_NONE));
        let mut out = [core::ptr::null_mut(); 8];
        let mut n: EGLint = -1;
        // SAFETY: an EGL_NONE-terminated list; room for 8 configs.
        let ok = unsafe { eglChooseConfig(dpy, attrs.as_ptr(), out.as_mut_ptr(), 8, &mut n) };
        if ok != TRUE {
            assert_eq!(n, -1, "num_config is untouched on failure");
            return Err(egl_error());
        }
        Ok(out[..n as usize].iter().map(|c| *c as usize).collect())
    }

    fn config_attr(dpy: EGLDisplay, config: EGLConfig, attribute: EGLint) -> EGLint {
        let mut v: EGLint = -12345;
        // SAFETY: `v` is writable.
        assert_eq!(unsafe { eglGetConfigAttrib(dpy, config, attribute, &mut v) }, TRUE, "0x{attribute:x}");
        v
    }

    fn query_surface(dpy: EGLDisplay, s: EGLSurface, attribute: EGLint) -> Result<EGLint, EGLint> {
        let mut v: EGLint = -12345;
        // SAFETY: `v` is writable.
        if unsafe { eglQuerySurface(dpy, s, attribute, &mut v) } == TRUE {
            Ok(v)
        } else {
            assert_eq!(v, -12345);
            Err(egl_error())
        }
    }

    fn query_context(dpy: EGLDisplay, c: EGLContext, attribute: EGLint) -> Result<EGLint, EGLint> {
        let mut v: EGLint = -12345;
        // SAFETY: `v` is writable.
        if unsafe { eglQueryContext(dpy, c, attribute, &mut v) } == TRUE {
            Ok(v)
        } else {
            assert_eq!(v, -12345);
            Err(egl_error())
        }
    }

    /// A new texture in the current GL context, bound, with w x h RGBA storage.
    fn texture(w: GLsizei, h: GLsizei) -> GLuint {
        let mut t: GLuint = 0;
        // SAFETY: room for one name; null pixels only allocate.
        unsafe {
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        }
        t
    }

    /// A renderbuffer with storage of the given format.
    fn renderbuffer(format: GLenum, w: GLsizei, h: GLsizei) -> GLuint {
        let mut rb: GLuint = 0;
        // SAFETY: room for one name; plain GL calls.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, format, w, h);
        }
        rb
    }

    /// A complete cube map: all six faces of level 0, RGBA8888.
    fn cube_texture(w: GLsizei, h: GLsizei) -> GLuint {
        let mut t: GLuint = 0;
        // SAFETY: room for one name; null pixels only allocate.
        unsafe {
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_CUBE_MAP, t);
            for f in 0..6 {
                let face = GL_TEXTURE_CUBE_MAP_POSITIVE_X + f;
                glTexImage2D(face, 0, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
            }
        }
        t
    }

    fn image_from_face(dpy: EGLDisplay, ctx: EGLContext, tex: GLuint, face: u32) -> EGLImage {
        let target = (EGL_GL_TEXTURE_CUBE_MAP_POSITIVE_X as EGLenum) + face;
        // SAFETY: the client buffer of a cube-map target is the texture name.
        unsafe { eglCreateImage(dpy, ctx, target, tex as usize as EGLClientBuffer, core::ptr::null()) }
    }

    fn image_from_texture(dpy: EGLDisplay, ctx: EGLContext, tex: GLuint) -> EGLImage {
        // SAFETY: the client buffer of EGL_GL_TEXTURE_2D is the texture name.
        unsafe {
            eglCreateImage(dpy, ctx, EGL_GL_TEXTURE_2D as EGLenum, tex as usize as EGLClientBuffer, core::ptr::null())
        }
    }

    fn fence(dpy: EGLDisplay) -> EGLSync {
        // SAFETY: plain EGL call.
        unsafe { eglCreateSync(dpy, EGL_SYNC_FENCE as EGLenum, core::ptr::null()) }
    }

    fn window(format: u32, w: u32, h: u32) -> GlesNativeWindow {
        let bpp = if format == GLES_NATIVE_FORMAT_RGB565 { 2 } else { 4 };
        GlesNativeWindow { phys: 0x8000_0000, width: w, height: h, stride_bytes: w * bpp, format }
    }

    fn window_surface(dpy: EGLDisplay, config: EGLConfig, win: &mut GlesNativeWindow) -> EGLSurface {
        // SAFETY: `win` is a valid native window description.
        unsafe { eglCreateWindowSurface(dpy, config, win, core::ptr::null()) }
    }

    /* ---- a platform with map_phys --------------------------------------------------
     * The mock OSAL has mmu_map only. These tests run on a copy of its table
     * with a map_phys that "maps" the caller's buffer (phys = its address). */

    static MAP_PHYS_FAILS: AtomicBool = AtomicBool::new(false);
    const MAPPED_VA: u64 = 0x90_0000_0000;

    unsafe extern "C" fn t_map_phys(_heap: u64, phys: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
        if MAP_PHYS_FAILS.load(Ordering::Relaxed) {
            return -crate::util::ENOMEM;
        }
        let cpu = phys as usize as *mut c_void;
        let gpu = MAPPED_VA + (phys & 0xffff_ffff);
        crate::testutil::mock_map_phys(gpu, size, cpu);
        // SAFETY: `out` is the caller's valid descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys, size, impl_: cpu } };
        0
    }

    /// Runs the test on a modified copy of the installed OSAL table.
    struct OpsSwap {
        saved: *const OsalOps,
    }

    impl OpsSwap {
        fn new(edit: impl FnOnce(&mut OsalOps)) -> Self {
            let base = osal::ops().expect("the mock OSAL");
            let mut ops = OsalOps { ..*base };
            edit(&mut ops);
            // SAFETY: tests are serialised by TestGl; the leaked table is 'static.
            unsafe { osal::osal_set_ops(Box::leak(Box::new(ops))) };
            MAP_PHYS_FAILS.store(false, Ordering::Relaxed);
            OpsSwap { saved: base }
        }
    }

    impl Drop for OpsSwap {
        fn drop(&mut self) {
            // SAFETY: the saved table is the mock's static.
            unsafe { osal::osal_set_ops(self.saved) };
        }
    }

    /* ---- pure helpers ------------------------------------------------------------------ */

    #[test]
    fn attrib_of_finds_a_key_or_returns_the_default() {
        let list = [EGL_WIDTH, 64, EGL_HEIGHT, 32, EGL_WIDTH, 99, EGL_NONE];
        // SAFETY: EGL_NONE-terminated lists / null.
        unsafe {
            assert_eq!(attrib_of(list.as_ptr(), EGL_WIDTH, -1), 99, "the last occurrence wins");
            assert_eq!(attrib_of(list.as_ptr(), EGL_HEIGHT, -1), 32);
            assert_eq!(attrib_of(list.as_ptr(), EGL_SAMPLES, -1), -1);
            assert_eq!(attrib_of([EGL_NONE].as_ptr(), EGL_WIDTH, 7), 7);
            assert_eq!(attrib_of(core::ptr::null(), EGL_WIDTH, 7), 7);
        }
    }

    #[test]
    fn attrib_of_matches_keys_not_values() {
        /* a value that happens to equal the key searched for is not a key */
        let list = [EGL_HEIGHT, EGL_WIDTH, EGL_WIDTH, 5, EGL_NONE];
        // SAFETY: an EGL_NONE-terminated list.
        assert_eq!(unsafe { attrib_of(list.as_ptr(), EGL_WIDTH, -1) }, 5);
    }

    #[test]
    fn native_formats_map_to_surface_formats() {
        assert_eq!(native_format_to_surf(GLES_NATIVE_FORMAT_RGBA8888), Some(GLES_SURF_RGBA8888));
        assert_eq!(native_format_to_surf(GLES_NATIVE_FORMAT_XRGB8888), Some(GLES_SURF_XRGB8888));
        assert_eq!(native_format_to_surf(GLES_NATIVE_FORMAT_RGB565), Some(GLES_SURF_RGB565));
        assert_eq!(native_format_to_surf(0), None);
        assert_eq!(native_format_to_surf(4), None);
        assert_eq!(native_format_to_surf(u32::MAX), None);
    }

    #[test]
    fn native_buffer_must_be_backed_non_empty_known_and_at_the_pair_pitch() {
        let ok = window(GLES_NATIVE_FORMAT_XRGB8888, 8, 4);
        assert_eq!(native_buffer_format(&ok), Some(GLES_SURF_XRGB8888));
        assert_eq!(native_buffer_format(&GlesNativeWindow { phys: 0, ..ok }), None);
        assert_eq!(native_buffer_format(&GlesNativeWindow { width: 0, stride_bytes: 0, ..ok }), None);
        assert_eq!(native_buffer_format(&GlesNativeWindow { height: 0, ..ok }), None);
        assert_eq!(native_buffer_format(&GlesNativeWindow { format: 9, ..ok }), None);
        /* a padded stride, and the stride of the wrong pixel size */
        assert_eq!(native_buffer_format(&GlesNativeWindow { stride_bytes: 36, ..ok }), None);
        assert_eq!(native_buffer_format(&GlesNativeWindow { stride_bytes: 16, ..ok }), None);
        assert_eq!(native_buffer_format(&GlesNativeWindow { format: GLES_NATIVE_FORMAT_RGB565, ..ok }), None);
        /* an odd width: the GPU writes and samples whole pixel pairs, so
         * the rows must carry the pad pixel (a packed buffer is refused
         * rather than written past its last row) */
        let odd = window(GLES_NATIVE_FORMAT_RGB565, 7, 3);
        assert_eq!(odd.stride_bytes, 14);
        assert_eq!(native_buffer_format(&odd), None);
        assert_eq!(native_buffer_format(&GlesNativeWindow { stride_bytes: 16, ..odd }), Some(GLES_SURF_RGB565));
        /* a size no allocation could hold */
        assert_eq!(native_buffer_format(&GlesNativeWindow { width: 20000, stride_bytes: 80000, ..ok }), None);
        assert_eq!(native_buffer_format(&GlesNativeWindow { height: 20000, ..ok }), None);
    }

    #[test]
    fn only_the_multisampled_config_has_four_samples() {
        assert_eq!(config_samples(CONFIG_1X), 1);
        assert_eq!(config_samples(CONFIG_4X), 4);
        assert_eq!(config_samples(CONFIG_565), 1);
    }

    #[test]
    fn valid_configs_are_the_three_handles() {
        assert!(valid_config(CONFIG_1X) && valid_config(CONFIG_565));
        assert_eq!(valid_config(CONFIG_4X), !cfg!(not(feature = "gpu-powervr")), "4x only where multisampling works");
        assert!(!valid_config(core::ptr::null_mut()));
        assert!(!valid_config(4 as EGLConfig));
    }

    #[test]
    fn config_buffer_size_is_the_sum_of_its_colour_components() {
        /* EGL 1.5 3.4: EGL_BUFFER_SIZE is the sum of the R, G, B and A sizes */
        for &cfg in offered_configs() {
            let get = |a| config_attrib(cfg, a).unwrap();
            assert_eq!(
                get(EGL_BUFFER_SIZE),
                get(EGL_RED_SIZE) + get(EGL_GREEN_SIZE) + get(EGL_BLUE_SIZE) + get(EGL_ALPHA_SIZE)
            );
        }
    }

    #[test]
    fn proc_table_names_are_nul_terminated_and_unique() {
        for (i, p) in PROCS.iter().enumerate() {
            assert_eq!(p.name.last(), Some(&0), "entry {i}");
            assert!(p.name.len() > 1 && !p.name[..p.name.len() - 1].contains(&0), "entry {i}");
            assert!(PROCS[..i].iter().all(|q| q.name != p.name), "{:?} twice", CStr::from_bytes_with_nul(p.name));
        }
    }

    /* ---- display --------------------------------------------------------------------------- */

    #[test]
    fn get_display_is_the_one_display_whatever_the_native_id() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::uninit(&gl);
        assert!(!egl.dpy.is_null());
        // SAFETY: plain EGL calls; the native display is only recorded.
        unsafe {
            assert_eq!(eglGetDisplay(0x1234 as EGLNativeDisplayType), egl.dpy);
            assert_eq!(eglGetPlatformDisplay(0, core::ptr::null_mut(), core::ptr::null()), egl.dpy);
        }
    }

    #[test]
    fn get_display_records_the_native_display_until_initialised() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::uninit(&gl);
        // SAFETY: plain EGL calls; the mock OSAL's init ignores its argument.
        unsafe {
            eglGetDisplay(0x1234 as EGLNativeDisplayType);
            assert_eq!(state().platform_arg as usize, 0x1234);
            assert_eq!(eglInitialize(egl.dpy, core::ptr::null_mut(), core::ptr::null_mut()), TRUE);
            eglGetDisplay(0x5678 as EGLNativeDisplayType);
            assert_eq!(state().platform_arg as usize, 0x1234, "an initialised display keeps its platform");
        }
    }

    #[test]
    fn initialize_reports_egl_1_5() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::uninit(&gl);
        let (mut major, mut minor) = (-1, -1);
        // SAFETY: major/minor are writable.
        unsafe {
            assert_eq!(eglInitialize(egl.dpy, &mut major, &mut minor), TRUE);
            assert_eq!((major, minor), (1, 5));
            assert_eq!(egl_error(), EGL_SUCCESS);
            /* again: still fine, same answer */
            (major, minor) = (-1, -1);
            assert_eq!(eglInitialize(egl.dpy, &mut major, &mut minor), TRUE);
            assert_eq!((major, minor), (1, 5));
            /* the version string agrees */
            let v = CStr::from_ptr(eglQueryString(egl.dpy, EGL_VERSION)).to_str().unwrap();
            assert!(v.starts_with("1.5"), "{v}");
        }
    }

    #[test]
    fn initialize_clears_a_stale_error() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::uninit(&gl);
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglBindAPI(EGL_OPENGL_API as EGLenum), FALSE);
            assert_eq!(eglInitialize(egl.dpy, core::ptr::null_mut(), core::ptr::null_mut()), TRUE);
        }
        assert_eq!(egl_error(), EGL_SUCCESS);
    }

    #[test]
    fn initialize_bad_display() {
        let gl = TestGl::new(8, 8);
        let _egl = Egl::uninit(&gl);
        let mut major = -1;
        // SAFETY: major is writable.
        assert_eq!(unsafe { eglInitialize(0x1 as EGLDisplay, &mut major, core::ptr::null_mut()) }, FALSE);
        assert_eq!(egl_error(), EGL_BAD_DISPLAY);
        assert_eq!(major, -1);
        assert!(!state().inited);
    }

    #[test]
    fn initialize_without_a_platform_or_backend_is_not_initialized() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::uninit(&gl);
        // SAFETY: tests are serialised; both tables are restored before anything can panic.
        let (no_osal, no_backend) = unsafe {
            let ops = osal::g_osal;
            osal::osal_set_ops(core::ptr::null());
            let a = (eglInitialize(egl.dpy, core::ptr::null_mut(), core::ptr::null_mut()), eglGetError());
            osal::osal_set_ops(ops);

            let be = backend::g_backend;
            backend::g_backend = core::ptr::null();
            let b = (eglInitialize(egl.dpy, core::ptr::null_mut(), core::ptr::null_mut()), eglGetError());
            backend::g_backend = be;
            (a, b)
        };
        assert_eq!(no_osal, (FALSE, EGL_NOT_INITIALIZED));
        assert_eq!(no_backend, (FALSE, EGL_NOT_INITIALIZED));
        assert!(!state().inited);
    }

    #[test]
    fn terminate_is_idempotent_and_initialize_works_again() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglTerminate(egl.dpy), TRUE);
            assert_eq!(eglTerminate(egl.dpy), TRUE);
            assert_eq!(egl_error(), EGL_SUCCESS);
            assert_eq!(eglInitialize(egl.dpy, core::ptr::null_mut(), core::ptr::null_mut()), TRUE);
        }
        assert!(!egl.pbuffer(CONFIG_1X, 4, 4).is_null());
    }

    #[test]
    fn terminate_before_initialize_and_with_a_bad_display() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::uninit(&gl);
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglTerminate(egl.dpy), TRUE);
            assert_eq!(egl_error(), EGL_SUCCESS);
            assert_eq!(eglTerminate(core::ptr::null_mut()), FALSE);
            assert_eq!(egl_error(), EGL_BAD_DISPLAY);
        }
    }

    #[test]
    fn after_terminate_the_display_is_not_initialized() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let mut n: EGLint = -1;
        // SAFETY: plain EGL calls; `n` is writable.
        unsafe {
            eglTerminate(egl.dpy);
            assert_eq!(eglGetConfigs(egl.dpy, core::ptr::null_mut(), 0, &mut n), FALSE);
            assert_eq!(egl_error(), EGL_NOT_INITIALIZED);
            assert!(eglQueryString(egl.dpy, EGL_VENDOR).is_null());
            assert_eq!(egl_error(), EGL_NOT_INITIALIZED);
        }
        assert!(egl.pbuffer(CONFIG_1X, 4, 4).is_null());
        assert_eq!(egl_error(), EGL_NOT_INITIALIZED);
        assert!(egl.context(CONFIG_1X).is_null());
        assert_eq!(egl_error(), EGL_NOT_INITIALIZED);
        assert_eq!(n, -1);
    }

    #[test]
    fn terminate_releases_surfaces_contexts_and_images() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (s, c) = egl.current(4, 4);
        let tex = texture(4, 4);
        let img = image_from_texture(egl.dpy, c, tex);
        assert!(!img.is_null());
        egl.pbuffer(CONFIG_1X, 2, 2);
        assert_eq!(gl.live_allocs(), 1 + 3, "TestGl's surface, two pbuffers, the texture");
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglTerminate(egl.dpy), TRUE);
            assert_eq!(eglGetCurrentContext(), NO_CONTEXT);
            assert_eq!(eglGetCurrentSurface(EGL_DRAW), NO_SURFACE);
            assert!(context::gl_ctx.is_null());
        }
        assert_eq!(gl.live_allocs(), 1);
        let e = state();
        assert!(e.surf.iter().all(|s| !s.used) && e.ctx.iter().all(|c| !c.used) && e.img.iter().all(|i| !i.used));
        /* the old handles are dead after a new eglInitialize */
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglInitialize(egl.dpy, core::ptr::null_mut(), core::ptr::null_mut()), TRUE);
            assert_eq!(eglMakeCurrent(egl.dpy, s, s, c), FALSE);
            assert_eq!(egl_error(), EGL_BAD_CONTEXT);
        }
        /* and the GL context slot is free again */
        egl.current(4, 4);
    }

    #[test]
    fn terminate_releases_sync_objects() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        egl.current(4, 4);
        for _ in 0..EGL_MAX_SYNCS {
            assert!(!fence(egl.dpy).is_null());
        }
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglTerminate(egl.dpy), TRUE);
            assert_eq!(eglInitialize(egl.dpy, core::ptr::null_mut(), core::ptr::null_mut()), TRUE);
        }
        egl.current(4, 4);
        assert!(!fence(egl.dpy).is_null(), "error 0x{:x}", egl_error());
    }

    #[test]
    fn get_error_returns_the_error_once() {
        let gl = TestGl::new(8, 8);
        let _egl = Egl::new(&gl);
        // SAFETY: plain EGL call.
        assert_eq!(unsafe { eglBindAPI(EGL_OPENVG_API as EGLenum) }, FALSE);
        assert_eq!(egl_error(), EGL_BAD_PARAMETER);
        assert_eq!(egl_error(), EGL_SUCCESS);
        assert_eq!(egl_error(), EGL_SUCCESS);
    }

    #[test]
    fn the_error_state_of_a_fresh_process_is_egl_success() {
        /* EGL 1.5 3.1: EGL_SUCCESS is 0x3000. A zero-initialised error made
         * the first eglGetError of a process report a code no client knows.
         *
         * The guard is taken for the reason every other test takes one, and
         * this test is why the rule is worth writing down: it used to call
         * `reset_for_tests()` holding nothing. That wipes the display, the
         * surface table and the current bindings, and the tests that own
         * those run in OTHER threads -- so about one run in a hundred it
         * landed inside one of them and an EGL call that should have
         * succeeded returned EGL_FALSE. `TestGl::new` resets the same state,
         * so what this measures is unchanged. */
        let _gl = TestGl::new(8, 8);
        assert_eq!(egl_error(), EGL_SUCCESS);
        assert_ne!(EGL_SUCCESS, 0);
    }

    #[test]
    fn a_call_that_succeeds_clears_the_error_of_the_one_before_it() {
        /* EGL 1.5 3.1: the error state reflects the last call, so one stale
         * error must not fail every later check against a call that worked. */
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglBindAPI(EGL_OPENVG_API as EGLenum), FALSE);
            assert_eq!(state().error, EGL_BAD_PARAMETER, "the failure is recorded");
            assert_eq!(eglQueryAPI(), EGL_OPENGL_ES_API as EGLenum);
        }
        assert_eq!(egl_error(), EGL_SUCCESS, "the query that succeeded cleared it");

        // SAFETY: plain EGL calls.
        unsafe {
            assert!(eglCreatePixmapSurface(egl.dpy, CONFIG_1X, core::ptr::null_mut(), core::ptr::null()).is_null());
            assert_eq!(state().error, EGL_BAD_MATCH);
            assert_eq!(eglGetDisplay(core::ptr::null_mut()), egl.dpy);
        }
        assert_eq!(egl_error(), EGL_SUCCESS);
    }

    #[test]
    fn query_string_contents() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        // SAFETY: eglQueryString returns static NUL-terminated strings.
        let get = |name| unsafe {
            let p = eglQueryString(egl.dpy, name);
            assert!(!p.is_null(), "0x{name:x}");
            CStr::from_ptr(p).to_str().unwrap()
        };
        assert!(get(EGL_VENDOR).contains("YakoGL"));
        assert!(get(EGL_VERSION).starts_with("1.5"));
        assert_eq!(get(EGL_CLIENT_APIS), "OpenGL_ES");
        let ext: Vec<&str> = get(EGL_EXTENSIONS).split(' ').collect();
        /* what eglCreateImage implements */
        /* ... and EGL_KHR_create_context, which dEQP asks for by name before
         * it makes an ES 3 context. Revert check: drop it from the string. */
        for want in
            ["EGL_KHR_image_base", "EGL_KHR_gl_texture_2D_image", "EGL_KHR_image_pixmap", "EGL_KHR_create_context"]
        {
            assert!(ext.contains(&want), "{want} missing from {ext:?}");
        }
        assert!(ext.iter().all(|e| e.starts_with("EGL_")), "{ext:?}");
        assert_eq!(egl_error(), EGL_SUCCESS);
    }

    #[test]
    fn query_string_errors() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        // SAFETY: plain EGL calls.
        unsafe {
            assert!(eglQueryString(egl.dpy, EGL_WIDTH).is_null());
            assert_eq!(egl_error(), EGL_BAD_PARAMETER);
            assert!(eglQueryString(0x1 as EGLDisplay, EGL_VENDOR).is_null());
            assert_eq!(egl_error(), EGL_BAD_DISPLAY);
            assert!(eglQueryString(NO_DISPLAY, EGL_VENDOR).is_null());
            assert_eq!(egl_error(), EGL_BAD_DISPLAY);
        }
    }

    #[test]
    fn query_string_client_extensions_need_no_display() {
        let gl = TestGl::new(8, 8);
        let _egl = Egl::uninit(&gl);
        // SAFETY: eglQueryString returns a static NUL-terminated string.
        let s = unsafe { CStr::from_ptr(eglQueryString(NO_DISPLAY, EGL_EXTENSIONS)) };
        /* the one client extension is the query itself; an empty string here
         * claims the mechanism without the name it is known by, which dEQP's
         * client_extensions.base reads as a contradiction */
        assert_eq!(s.to_bytes(), b"EGL_EXT_client_extensions");
        assert_eq!(egl_error(), EGL_SUCCESS);
        /* the client and display extension strings must stay disjoint */
        let egl = Egl::new(&gl);
        // SAFETY: as above.
        let d = unsafe { CStr::from_ptr(eglQueryString(egl.dpy, EGL_EXTENSIONS)) };
        assert!(!d.to_bytes().windows(s.to_bytes().len()).any(|w| w == s.to_bytes()));
    }

    #[test]
    fn bind_api_accepts_only_opengl_es() {
        let gl = TestGl::new(8, 8);
        let _egl = Egl::uninit(&gl);
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglQueryAPI(), EGL_OPENGL_ES_API as EGLenum, "the initial API");
            assert_eq!(eglBindAPI(EGL_OPENGL_ES_API as EGLenum), TRUE);
            assert_eq!(egl_error(), EGL_SUCCESS);
            for api in [EGL_OPENGL_API, EGL_OPENVG_API, 0] {
                assert_eq!(eglBindAPI(api as EGLenum), FALSE);
                assert_eq!(egl_error(), EGL_BAD_PARAMETER);
            }
            assert_eq!(eglQueryAPI(), EGL_OPENGL_ES_API as EGLenum);
        }
    }

    #[test]
    fn release_thread_succeeds_and_keeps_the_display_initialised() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        // SAFETY: plain EGL call.
        assert_eq!(unsafe { eglReleaseThread() }, TRUE);
        assert_eq!(egl_error(), EGL_SUCCESS);
        assert!(!egl.pbuffer(CONFIG_1X, 4, 4).is_null());
    }

    /* ---- configs ------------------------------------------------------------------------------ */

    /// `ids` as this build offers them: the GC7000 build has no 4x config
    /// (offered_configs()), so an expected list names it only where it exists.
    fn offered(ids: &[usize]) -> std::vec::Vec<usize> {
        ids.iter().copied().filter(|&i| valid_config(i as EGLConfig)).collect()
    }

    /// An 8888 config other than CONFIG_1X where the build has one, for tests
    /// that only need "an RGBA8888 config", not a multisampled one.
    const CONFIG_4X_OR_1X: EGLConfig = if cfg!(not(feature = "gpu-powervr")) { CONFIG_1X } else { CONFIG_4X };

    /// The GC7000 build offers no config it cannot honour: its backend has
    /// multisampling off, so there is no 4x config to list, choose or make a
    /// surface on. The V3D build neither: its backend does not multisample.
    #[cfg(not(feature = "gpu-powervr"))]
    #[test]
    fn the_gc7000_offers_no_multisampled_config() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        assert!(!valid_config(CONFIG_4X));
        let mut n: EGLint = -1;
        // SAFETY: `n` is writable; a null array only counts.
        unsafe { assert_eq!(eglGetConfigs(egl.dpy, core::ptr::null_mut(), 0, &mut n), TRUE) };
        assert_eq!(n, 2);
        assert_eq!(choose(egl.dpy, &[EGL_SAMPLES, 4, EGL_NONE]), Ok(std::vec![]));
        assert_eq!(choose(egl.dpy, &[EGL_SAMPLE_BUFFERS, 1, EGL_NONE]), Ok(std::vec![]));
        assert!(egl.pbuffer(CONFIG_4X, 4, 4).is_null(), "no surface on a config not offered");
        assert_eq!(egl_error(), EGL_BAD_CONFIG);
    }

    /// ...and offers it again while the backend multisamples: the switch
    /// the MSAA measurement turns on brings the config back with it, so a
    /// 4x surface is one the draw can honour.
    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn the_gc7000_offers_the_multisampled_config_while_msaa_is_on() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        crate::vivante::backend::gc_debug_set_msaa(1);
        let on = (valid_config(CONFIG_4X), choose(egl.dpy, &[EGL_SAMPLES, 4, EGL_NONE]), egl.pbuffer(CONFIG_4X, 16, 4));
        crate::vivante::backend::gc_debug_set_msaa(0);
        assert!(on.0);
        assert_eq!(on.1, Ok(std::vec![2]));
        assert!(!on.2.is_null());
        assert_eq!(surf(on.2).samples, 4);
        assert!(!valid_config(CONFIG_4X), "and gone again when it is off");
    }

    /// The V3D build the same, with its own switch (MSAA step 3): the 4x
    /// config while `v3d_debug_msaa` is on, and a 4x pbuffer on it.
    #[cfg(feature = "gpu-v3d")]
    #[test]
    fn the_v3d_offers_the_multisampled_config_while_msaa_is_on() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        crate::v3d::backend::v3d_debug_set_msaa(1);
        let on = (valid_config(CONFIG_4X), choose(egl.dpy, &[EGL_SAMPLES, 4, EGL_NONE]), egl.pbuffer(CONFIG_4X, 16, 4));
        crate::v3d::backend::v3d_debug_set_msaa(0);
        assert!(on.0);
        assert_eq!(on.1, Ok(std::vec![2]));
        assert!(!on.2.is_null());
        assert_eq!(surf(on.2).samples, 4);
        assert!(!valid_config(CONFIG_4X), "and gone again when it is off");
    }

    #[test]
    fn get_configs_counts_three_and_fills_up_to_config_size() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let mut n: EGLint = -1;
        // SAFETY: `n` is writable; the arrays have room for config_size entries.
        unsafe {
            let all = offered_configs().len() as EGLint;
            assert_eq!(eglGetConfigs(egl.dpy, core::ptr::null_mut(), 0, &mut n), TRUE);
            assert_eq!(n, all);
            assert_eq!(eglGetConfigs(egl.dpy, core::ptr::null_mut(), 1, &mut n), TRUE);
            assert_eq!(n, all, "a null array only counts");

            let mut out = [core::ptr::null_mut(); 5];
            assert_eq!(eglGetConfigs(egl.dpy, out.as_mut_ptr(), 5, &mut n), TRUE);
            assert_eq!(n, all);
            assert_eq!(out[..offered_configs().len()], *offered_configs());
            assert!(out[offered_configs().len()..].iter().all(|c| c.is_null()));

            let mut out = [core::ptr::null_mut(); 5];
            assert_eq!(eglGetConfigs(egl.dpy, out.as_mut_ptr(), 2, &mut n), TRUE);
            assert_eq!(n, 2);
            assert_eq!(out[..3], [offered_configs()[0], offered_configs()[1], core::ptr::null_mut()]);

            let mut out = [core::ptr::null_mut(); 5];
            assert_eq!(eglGetConfigs(egl.dpy, out.as_mut_ptr(), 0, &mut n), TRUE);
            assert_eq!(n, 0);
            assert!(out[0].is_null());
        }
        assert_eq!(egl_error(), EGL_SUCCESS);
    }

    #[test]
    fn get_configs_errors() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let mut n: EGLint = -1;
        // SAFETY: `n` is writable.
        unsafe {
            assert_eq!(eglGetConfigs(egl.dpy, core::ptr::null_mut(), 0, core::ptr::null_mut()), FALSE);
            assert_eq!(egl_error(), EGL_BAD_PARAMETER);
            assert_eq!(eglGetConfigs(0x1 as EGLDisplay, core::ptr::null_mut(), 0, &mut n), FALSE);
            assert_eq!(egl_error(), EGL_BAD_DISPLAY);
        }
        assert_eq!(n, -1);
    }

    #[test]
    fn config_attribs_of_the_rgba8888_config() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let a = |attr| config_attr(egl.dpy, CONFIG_1X, attr);
        assert_eq!((a(EGL_RED_SIZE), a(EGL_GREEN_SIZE), a(EGL_BLUE_SIZE), a(EGL_ALPHA_SIZE)), (8, 8, 8, 8));
        assert_eq!(a(EGL_BUFFER_SIZE), 32);
        assert_eq!((a(EGL_DEPTH_SIZE), a(EGL_STENCIL_SIZE)), (24, 8));
        assert_eq!((a(EGL_SAMPLES), a(EGL_SAMPLE_BUFFERS)), (0, 0));
        assert_eq!(a(EGL_CONFIG_ID), 1);
    }

    #[cfg(feature = "gpu-powervr")] /* the GC7000 build has no 4x config */
    #[test]
    fn config_attribs_of_the_multisampled_config() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let a = |attr| config_attr(egl.dpy, CONFIG_4X, attr);
        assert_eq!((a(EGL_RED_SIZE), a(EGL_GREEN_SIZE), a(EGL_BLUE_SIZE), a(EGL_ALPHA_SIZE)), (8, 8, 8, 8));
        assert_eq!(a(EGL_BUFFER_SIZE), 32);
        assert_eq!((a(EGL_DEPTH_SIZE), a(EGL_STENCIL_SIZE)), (24, 8));
        assert_eq!((a(EGL_SAMPLES), a(EGL_SAMPLE_BUFFERS)), (4, 1));
        assert_eq!(a(EGL_CONFIG_ID), 2);
    }

    #[test]
    fn config_attribs_of_the_rgb565_config() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let a = |attr| config_attr(egl.dpy, CONFIG_565, attr);
        assert_eq!((a(EGL_RED_SIZE), a(EGL_GREEN_SIZE), a(EGL_BLUE_SIZE), a(EGL_ALPHA_SIZE)), (5, 6, 5, 0));
        assert_eq!(a(EGL_BUFFER_SIZE), 16);
        assert_eq!((a(EGL_DEPTH_SIZE), a(EGL_STENCIL_SIZE)), (24, 8));
        assert_eq!((a(EGL_SAMPLES), a(EGL_SAMPLE_BUFFERS)), (0, 0));
        assert_eq!(a(EGL_CONFIG_ID), 3);
    }

    #[test]
    fn config_attribs_common_to_every_config() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        for &cfg in offered_configs() {
            let a = |attr| config_attr(egl.dpy, cfg, attr);
            /* the colour buffer survives a swap, so the config says so and
             * eglSurfaceAttrib takes EGL_BUFFER_PRESERVED (dEQP checks the
             * two answers against each other) */
            assert_eq!(a(EGL_SURFACE_TYPE), EGL_WINDOW_BIT | EGL_PBUFFER_BIT | EGL_SWAP_BEHAVIOR_PRESERVED_BIT);
            assert_eq!(a(EGL_RENDERABLE_TYPE), EGL_OPENGL_ES2_BIT | EGL_OPENGL_ES3_BIT);
            assert_eq!(
                a(EGL_CONFORMANT),
                EGL_OPENGL_ES2_BIT,
                "EGL_CONFORMANT claims ES 3 while glBlitFramebuffer always fails (review R-13)"
            );
            assert_eq!(a(EGL_COLOR_BUFFER_TYPE), EGL_RGB_BUFFER);
            assert_eq!(a(EGL_CONFIG_CAVEAT), EGL_NONE);
            assert_eq!(a(EGL_TRANSPARENT_TYPE), EGL_NONE);
            assert_eq!(a(EGL_NATIVE_RENDERABLE), EGL_TRUE);
            assert_eq!(a(EGL_NATIVE_VISUAL_ID), 0);
            assert_eq!(a(EGL_NATIVE_VISUAL_TYPE), EGL_NONE);
            assert_eq!(a(EGL_LEVEL), 0);
            assert_eq!((a(EGL_LUMINANCE_SIZE), a(EGL_ALPHA_MASK_SIZE)), (0, 0));
            assert_eq!((a(EGL_BIND_TO_TEXTURE_RGB), a(EGL_BIND_TO_TEXTURE_RGBA)), (EGL_FALSE, EGL_FALSE));
            /* what eglCreatePbufferSurface accepts */
            assert_eq!((a(EGL_MAX_PBUFFER_WIDTH), a(EGL_MAX_PBUFFER_HEIGHT)), (4096, 4096));
            assert_eq!(a(EGL_MAX_PBUFFER_PIXELS), 4096 * 4096);
            assert_eq!((a(EGL_MIN_SWAP_INTERVAL), a(EGL_MAX_SWAP_INTERVAL)), (0, 1));
            assert_eq!(
                (a(EGL_TRANSPARENT_RED_VALUE), a(EGL_TRANSPARENT_GREEN_VALUE), a(EGL_TRANSPARENT_BLUE_VALUE)),
                (0, 0, 0)
            );
        }
        assert_eq!(egl_error(), EGL_SUCCESS);
    }

    #[test]
    fn get_config_attrib_errors_leave_the_value_alone() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let mut v: EGLint = -7;
        // SAFETY: `v` is writable; null is rejected.
        unsafe {
            assert_eq!(eglGetConfigAttrib(egl.dpy, core::ptr::null_mut(), EGL_RED_SIZE, &mut v), FALSE);
            assert_eq!(egl_error(), EGL_BAD_CONFIG);
            assert_eq!(eglGetConfigAttrib(egl.dpy, 4 as EGLConfig, EGL_RED_SIZE, &mut v), FALSE);
            assert_eq!(egl_error(), EGL_BAD_CONFIG);
            assert_eq!(eglGetConfigAttrib(egl.dpy, CONFIG_1X, EGL_RED_SIZE, core::ptr::null_mut()), FALSE);
            assert_eq!(egl_error(), EGL_BAD_PARAMETER);
            assert_eq!(eglGetConfigAttrib(egl.dpy, CONFIG_1X, EGL_WIDTH, &mut v), FALSE);
            assert_eq!(egl_error(), EGL_BAD_ATTRIBUTE);
            assert_eq!(eglGetConfigAttrib(0x1 as EGLDisplay, CONFIG_1X, EGL_RED_SIZE, &mut v), FALSE);
            assert_eq!(egl_error(), EGL_BAD_DISPLAY);
        }
        assert_eq!(v, -7);
    }

    #[test]
    fn choose_config_without_criteria_returns_all_in_the_specs_order() {
        /* EGL 1.5 3.4.1: nothing asked for, so the colour bits are not
         * considered and the smaller EGL_BUFFER_SIZE leads. */
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        assert_eq!(choose(egl.dpy, &[EGL_NONE]), Ok(offered(&[3, 1, 2])));
        let mut n: EGLint = -1;
        let mut out = [core::ptr::null_mut(); 3];
        // SAFETY: a null list is allowed; room for three configs.
        unsafe { assert_eq!(eglChooseConfig(egl.dpy, core::ptr::null(), out.as_mut_ptr(), 3, &mut n), TRUE) };
        let want: std::vec::Vec<EGLConfig> = offered(&[3, 1, 2]).into_iter().map(|i| i as EGLConfig).collect();
        assert_eq!(n as usize, want.len());
        assert_eq!(out[..want.len()], want[..]);
    }

    #[test]
    fn choose_config_colour_sizes_are_at_least() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        /* The order is EGL 1.5 3.4.1's. With no colour size asked for, rule 3
         * contributes nothing and rule 4 decides: smaller EGL_BUFFER_SIZE
         * first, so 565 leads, then the two 8888s by EGL_SAMPLE_BUFFERS.
         * Ask for a colour size and rule 3 reverses that: the deeper buffer
         * wins on the total of the components that were asked for. */
        let all = Ok(offered(&[3, 1, 2]));
        let all_deep = Ok(offered(&[1, 2, 3]));
        let rgba = Ok(offered(&[1, 2]));
        let none = Ok(std::vec![]);
        assert_eq!(choose(egl.dpy, &[EGL_RED_SIZE, 5, EGL_GREEN_SIZE, 6, EGL_BLUE_SIZE, 5, EGL_NONE]), all_deep);
        assert_eq!(choose(egl.dpy, &[EGL_RED_SIZE, 1, EGL_NONE]), all_deep);
        /* zero is "not considered", so these do not reorder anything */
        assert_eq!(choose(egl.dpy, &[EGL_RED_SIZE, 0, EGL_ALPHA_SIZE, 0, EGL_NONE]), all);
        assert_eq!(choose(egl.dpy, &[EGL_RED_SIZE, 6, EGL_NONE]), rgba);
        assert_eq!(choose(egl.dpy, &[EGL_GREEN_SIZE, 7, EGL_NONE]), rgba);
        assert_eq!(choose(egl.dpy, &[EGL_BLUE_SIZE, 8, EGL_NONE]), rgba);
        assert_eq!(choose(egl.dpy, &[EGL_ALPHA_SIZE, 1, EGL_NONE]), rgba);
        assert_eq!(choose(egl.dpy, &[EGL_RED_SIZE, 9, EGL_NONE]), none);
        assert_eq!(choose(egl.dpy, &[EGL_BUFFER_SIZE, 16, EGL_NONE]), all);
        assert_eq!(choose(egl.dpy, &[EGL_BUFFER_SIZE, 17, EGL_NONE]), rgba);
        assert_eq!(choose(egl.dpy, &[EGL_BUFFER_SIZE, 33, EGL_NONE]), none);
        assert_eq!(choose(egl.dpy, &[EGL_LUMINANCE_SIZE, 1, EGL_NONE]), none);
        assert_eq!(choose(egl.dpy, &[EGL_ALPHA_MASK_SIZE, 1, EGL_NONE]), none);
    }

    #[test]
    fn choose_config_depth_and_stencil_are_at_least() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let all = Ok(offered(&[3, 1, 2]));
        assert_eq!(choose(egl.dpy, &[EGL_DEPTH_SIZE, 16, EGL_STENCIL_SIZE, 1, EGL_NONE]), all);
        assert_eq!(choose(egl.dpy, &[EGL_DEPTH_SIZE, 24, EGL_STENCIL_SIZE, 8, EGL_NONE]), all);
        assert_eq!(choose(egl.dpy, &[EGL_DEPTH_SIZE, 25, EGL_NONE]), Ok(std::vec![]));
        assert_eq!(choose(egl.dpy, &[EGL_STENCIL_SIZE, 9, EGL_NONE]), Ok(std::vec![]));
    }

    #[cfg(feature = "gpu-powervr")] /* the GC7000 build has no 4x config */
    #[test]
    fn choose_config_samples_select_the_multisampled_config() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        assert_eq!(choose(egl.dpy, &[EGL_SAMPLES, 4, EGL_NONE]), Ok(std::vec![2]));
        assert_eq!(choose(egl.dpy, &[EGL_SAMPLES, 2, EGL_NONE]), Ok(std::vec![2]));
        assert_eq!(choose(egl.dpy, &[EGL_SAMPLE_BUFFERS, 1, EGL_NONE]), Ok(std::vec![2]));
        assert_eq!(choose(egl.dpy, &[EGL_SAMPLES, 5, EGL_NONE]), Ok(std::vec![]));
        /* without a request the single-sampled config sorts first */
        assert_eq!(choose(egl.dpy, &[EGL_SAMPLES, 0, EGL_NONE]), Ok(offered(&[3, 1, 2])));
    }

    #[test]
    fn choose_config_dont_care_skips_the_attribute() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let list = [
            EGL_SAMPLES,
            EGL_DONT_CARE,
            EGL_CONFIG_ID,
            EGL_DONT_CARE,
            EGL_SURFACE_TYPE,
            EGL_DONT_CARE,
            EGL_MAX_SWAP_INTERVAL,
            EGL_DONT_CARE,
            EGL_COLOR_BUFFER_TYPE,
            EGL_DONT_CARE,
            EGL_NONE,
        ];
        assert_eq!(choose(egl.dpy, &list), Ok(offered(&[3, 1, 2])));
    }

    #[test]
    fn choose_config_bitmasks_must_be_subsets() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let all = Ok(offered(&[3, 1, 2]));
        let none = Ok(std::vec![]);
        assert_eq!(choose(egl.dpy, &[EGL_SURFACE_TYPE, EGL_WINDOW_BIT, EGL_NONE]), all);
        assert_eq!(choose(egl.dpy, &[EGL_SURFACE_TYPE, EGL_WINDOW_BIT | EGL_PBUFFER_BIT, EGL_NONE]), all);
        assert_eq!(choose(egl.dpy, &[EGL_SURFACE_TYPE, 0, EGL_NONE]), all);
        assert_eq!(choose(egl.dpy, &[EGL_SURFACE_TYPE, EGL_PIXMAP_BIT, EGL_NONE]), none);
        assert_eq!(choose(egl.dpy, &[EGL_SURFACE_TYPE, EGL_WINDOW_BIT | EGL_PIXMAP_BIT, EGL_NONE]), none);
        assert_eq!(choose(egl.dpy, &[EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE]), all);
        assert_eq!(choose(egl.dpy, &[EGL_RENDERABLE_TYPE, EGL_OPENGL_ES3_BIT, EGL_NONE]), all);
        assert_eq!(choose(egl.dpy, &[EGL_RENDERABLE_TYPE, EGL_OPENGL_BIT, EGL_NONE]), none);
        assert_eq!(choose(egl.dpy, &[EGL_RENDERABLE_TYPE, EGL_OPENGL_ES_BIT, EGL_NONE]), none, "no ES 1.x");
        assert_eq!(choose(egl.dpy, &[EGL_CONFORMANT, EGL_OPENGL_ES2_BIT, EGL_NONE]), all);
        /* ES 3 contexts are renderable but not conformant (R-13) */
        assert_eq!(choose(egl.dpy, &[EGL_CONFORMANT, EGL_OPENGL_ES3_BIT, EGL_NONE]), none);
        assert_eq!(choose(egl.dpy, &[EGL_CONFORMANT, EGL_OPENGL_ES2_BIT | EGL_OPENGL_ES3_BIT, EGL_NONE]), none);
        assert_eq!(choose(egl.dpy, &[EGL_CONFORMANT, EGL_OPENVG_BIT, EGL_NONE]), none);
    }

    #[test]
    fn choose_config_id_is_exact() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        for id in 1..=3 {
            assert_eq!(choose(egl.dpy, &[EGL_CONFIG_ID, id, EGL_NONE]), Ok(offered(&[id as usize])));
        }
        assert_eq!(choose(egl.dpy, &[EGL_CONFIG_ID, 0, EGL_NONE]), Ok(std::vec![]));
        assert_eq!(choose(egl.dpy, &[EGL_CONFIG_ID, 4, EGL_NONE]), Ok(std::vec![]));
    }

    #[test]
    fn choose_config_exact_attributes() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let all = Ok(offered(&[3, 1, 2]));
        let none = Ok(std::vec![]);
        assert_eq!(choose(egl.dpy, &[EGL_COLOR_BUFFER_TYPE, EGL_RGB_BUFFER, EGL_NONE]), all);
        assert_eq!(choose(egl.dpy, &[EGL_COLOR_BUFFER_TYPE, EGL_LUMINANCE_BUFFER, EGL_NONE]), none);
        assert_eq!(choose(egl.dpy, &[EGL_BIND_TO_TEXTURE_RGBA, EGL_FALSE, EGL_NONE]), all);
        assert_eq!(choose(egl.dpy, &[EGL_BIND_TO_TEXTURE_RGBA, EGL_TRUE, EGL_NONE]), none);
        assert_eq!(choose(egl.dpy, &[EGL_BIND_TO_TEXTURE_RGB, EGL_TRUE, EGL_NONE]), none);
        assert_eq!(choose(egl.dpy, &[EGL_TRANSPARENT_TYPE, EGL_NONE, EGL_NONE]), all);
        assert_eq!(choose(egl.dpy, &[EGL_TRANSPARENT_TYPE, EGL_TRANSPARENT_RGB, EGL_NONE]), none);
        assert_eq!(choose(egl.dpy, &[EGL_MIN_SWAP_INTERVAL, 0, EGL_MAX_SWAP_INTERVAL, 1, EGL_NONE]), all);
        assert_eq!(choose(egl.dpy, &[EGL_MIN_SWAP_INTERVAL, 1, EGL_NONE]), none);
        assert_eq!(choose(egl.dpy, &[EGL_MAX_SWAP_INTERVAL, -2, EGL_NONE]), none);
    }

    #[test]
    fn choose_config_takes_the_last_value_of_a_repeated_attribute() {
        /* The attribute list is a map. dEQP's choose_config.random names
         * EGL_BUFFER_SIZE several times in one list and expects the last
         * value to decide; taking every occurrence as a constraint answered
         * with too few configs, or none. */
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let all = Ok(offered(&[3, 1, 2]));
        assert_eq!(choose(egl.dpy, &[EGL_BUFFER_SIZE, 32, EGL_BUFFER_SIZE, 12, EGL_NONE]), all);
        assert_eq!(choose(egl.dpy, &[EGL_BUFFER_SIZE, 12, EGL_BUFFER_SIZE, 32, EGL_NONE]), Ok(offered(&[1, 2])));
        /* the sort order reads the list the same way: EGL 1.5 3.4.1 rule 3
         * adds up the colour bits of the components asked for, and a size of
         * 0 asks for none, so this order is by EGL_BUFFER_SIZE (rule 4) */
        assert_eq!(choose(egl.dpy, &[EGL_RED_SIZE, 4, EGL_NONE]), Ok(offered(&[1, 2, 3])), "red counts");
        assert_eq!(choose(egl.dpy, &[EGL_RED_SIZE, 4, EGL_RED_SIZE, 0, EGL_NONE]), all, "red no longer counts");
    }

    #[test]
    fn choose_config_attributes_with_nothing_to_choose_between_match_their_value() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let list = [
            EGL_LEVEL,
            0,
            EGL_NATIVE_RENDERABLE,
            EGL_TRUE,
            EGL_CONFIG_CAVEAT,
            EGL_NONE,
            EGL_NATIVE_VISUAL_TYPE,
            EGL_NONE,
            EGL_TRANSPARENT_RED_VALUE,
            0,
            EGL_MATCH_NATIVE_PIXMAP,
            EGL_NONE,
            EGL_NONE,
        ];
        assert_eq!(choose(egl.dpy, &list), Ok(offered(&[3, 1, 2])));
    }

    #[test]
    fn choose_config_a_typical_window_request_prefers_the_single_sampled_rgba_config() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let list = [
            EGL_SURFACE_TYPE,
            EGL_WINDOW_BIT,
            EGL_RENDERABLE_TYPE,
            EGL_OPENGL_ES2_BIT,
            EGL_RED_SIZE,
            8,
            EGL_GREEN_SIZE,
            8,
            EGL_BLUE_SIZE,
            8,
            EGL_ALPHA_SIZE,
            8,
            EGL_DEPTH_SIZE,
            24,
            EGL_STENCIL_SIZE,
            8,
            EGL_NONE,
        ];
        assert_eq!(choose(egl.dpy, &list), Ok(offered(&[1, 2])));
    }

    #[test]
    fn choose_config_unknown_attribute_is_bad_attribute() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        assert_eq!(choose(egl.dpy, &[EGL_WIDTH, 8, EGL_NONE]), Err(EGL_BAD_ATTRIBUTE));
        assert_eq!(choose(egl.dpy, &[EGL_RED_SIZE, 8, 0x1234, 1, EGL_NONE]), Err(EGL_BAD_ATTRIBUTE));
        /* EGL 1.5 table 3.4 gives these attributes a set of values, and a
         * value outside it is an error, not a config that cannot match */
        assert_eq!(choose(egl.dpy, &[EGL_BIND_TO_TEXTURE_RGB, 4, EGL_NONE]), Err(EGL_BAD_ATTRIBUTE));
        assert_eq!(choose(egl.dpy, &[EGL_BIND_TO_TEXTURE_RGBA, 5, EGL_NONE]), Err(EGL_BAD_ATTRIBUTE));
        assert_eq!(choose(egl.dpy, &[EGL_COLOR_BUFFER_TYPE, 0, EGL_NONE]), Err(EGL_BAD_ATTRIBUTE));
        assert_eq!(choose(egl.dpy, &[EGL_NATIVE_RENDERABLE, 6, EGL_NONE]), Err(EGL_BAD_ATTRIBUTE));
        assert_eq!(choose(egl.dpy, &[EGL_TRANSPARENT_TYPE, 6, EGL_NONE]), Err(EGL_BAD_ATTRIBUTE));
        assert_eq!(choose(egl.dpy, &[EGL_CONFIG_CAVEAT, 6, EGL_NONE]), Err(EGL_BAD_ATTRIBUTE));
        /* a name with no value behind it: dEQP passes {0xffffffff}, which is
         * also EGL_DONT_CARE's bit pattern */
        let mut n: EGLint = -1;
        // SAFETY: the walk stops at the bad name without reading past it.
        let ok = unsafe { eglChooseConfig(egl.dpy, [EGL_DONT_CARE].as_ptr(), core::ptr::null_mut(), 0, &mut n) };
        assert_eq!((ok, egl_error()), (FALSE, EGL_BAD_ATTRIBUTE));
    }

    /// Runs `f` on a copy of `list` whose last element is the last readable
    /// int before an inaccessible page, so a walk that reads one element past
    /// the EGL_NONE terminator faults instead of reading the caller's stack.
    #[cfg(target_os = "linux")]
    fn with_list_at_page_end(list: &[EGLint], f: impl FnOnce(*const EGLint)) {
        use core::ffi::c_void;
        extern "C" {
            fn mmap(addr: *mut c_void, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut c_void;
            fn mprotect(addr: *mut c_void, len: usize, prot: i32) -> i32;
            fn munmap(addr: *mut c_void, len: usize) -> i32;
        }
        const HALF: usize = 64 * 1024; /* a multiple of every Linux page size */
        const PROT_NONE: i32 = 0;
        const PROT_RW: i32 = 1 | 2;
        const MAP_PRIVATE_ANONYMOUS: i32 = 0x02 | 0x20;
        // SAFETY: a fresh anonymous mapping; its second half is made
        // inaccessible and the list is copied to the end of the first half.
        unsafe {
            let base = mmap(core::ptr::null_mut(), 2 * HALF, PROT_RW, MAP_PRIVATE_ANONYMOUS, -1, 0);
            assert_ne!(base as isize, -1, "mmap");
            assert_eq!(mprotect(base.cast::<u8>().add(HALF).cast(), HALF, PROT_NONE), 0, "mprotect");
            let dst = base.cast::<u8>().add(HALF).cast::<EGLint>().sub(list.len());
            core::ptr::copy_nonoverlapping(list.as_ptr(), dst, list.len());
            f(dst);
            munmap(base, 2 * HALF);
        }
    }

    /// The attribute-list walks read nothing behind the EGL_NONE terminator:
    /// requested_color_bits used to read a value for it, one int past the
    /// end of the caller's array (ASan: stack-buffer-overflow in the host
    /// harness, whose list ends `EGL_SAMPLES, n, EGL_NONE`). The guard page
    /// turns that read into a SIGSEGV where it is executed: at opt-level 0
    /// and under ASan. At opt-level 1 LLVM sank the unused load below the
    /// EGL_NONE test, so this profile alone did not see it.
    #[cfg(target_os = "linux")]
    #[test]
    fn choose_config_reads_nothing_past_the_terminator() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let all_offered = choose(egl.dpy, &[EGL_NONE]).unwrap();
        let supported = [
            EGL_BUFFER_SIZE,
            EGL_RED_SIZE,
            EGL_GREEN_SIZE,
            EGL_BLUE_SIZE,
            EGL_ALPHA_SIZE,
            EGL_LUMINANCE_SIZE,
            EGL_ALPHA_MASK_SIZE,
            EGL_DEPTH_SIZE,
            EGL_STENCIL_SIZE,
            EGL_SAMPLES,
            EGL_SAMPLE_BUFFERS,
            EGL_SURFACE_TYPE,
            EGL_RENDERABLE_TYPE,
            EGL_CONFORMANT,
            EGL_CONFIG_ID,
            EGL_CONFIG_CAVEAT,
            EGL_COLOR_BUFFER_TYPE,
            EGL_BIND_TO_TEXTURE_RGB,
            EGL_BIND_TO_TEXTURE_RGBA,
            EGL_LEVEL,
            EGL_MATCH_NATIVE_PIXMAP,
            EGL_MAX_SWAP_INTERVAL,
            EGL_MIN_SWAP_INTERVAL,
            EGL_NATIVE_RENDERABLE,
            EGL_NATIVE_VISUAL_ID,
            EGL_NATIVE_VISUAL_TYPE,
            EGL_TRANSPARENT_TYPE,
            EGL_TRANSPARENT_RED_VALUE,
            EGL_TRANSPARENT_GREEN_VALUE,
            EGL_TRANSPARENT_BLUE_VALUE,
        ];
        let run = |list: &[EGLint]| -> ([bool; 4], Result<Vec<usize>, EGLint>) {
            let mut got = ([false; 4], Err(0));
            with_list_at_page_end(list, |p| {
                // SAFETY: `p` is an EGL_NONE-terminated list.
                got.0 = unsafe { requested_color_bits(p) };
                let mut out = [core::ptr::null_mut(); 8];
                let mut n: EGLint = -1;
                // SAFETY: as above; room for 8 configs.
                got.1 = if unsafe { eglChooseConfig(egl.dpy, p, out.as_mut_ptr(), 8, &mut n) } == TRUE {
                    Ok(out[..n as usize].iter().map(|c| *c as usize).collect())
                } else {
                    Err(egl_error())
                };
            });
            got
        };
        /* the empty list */
        assert_eq!(run(&[EGL_NONE]), ([false; 4], Ok(all_offered.clone())));
        /* the host harness's list, which overflowed */
        let harness = run(&[EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_SAMPLES, 0, EGL_NONE]);
        assert_eq!(
            harness,
            ([false; 4], choose(egl.dpy, &[EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_SAMPLES, 0, EGL_NONE]))
        );
        assert!(harness.1.is_ok_and(|v| !v.is_empty()));
        /* every supported attribute on its own, and all of them at once */
        for &a in &supported {
            assert_eq!(run(&[a, EGL_DONT_CARE, EGL_NONE]), ([false; 4], Ok(all_offered.clone())), "0x{a:x}");
        }
        let mut list: Vec<EGLint> = supported.iter().flat_map(|&a| [a, EGL_DONT_CARE]).collect();
        list.extend_from_slice(&[EGL_GREEN_SIZE, 1, EGL_ALPHA_SIZE, 1, EGL_NONE]);
        let (want, got) = run(&list);
        assert_eq!(want, [false, true, false, true]);
        assert_eq!(got, choose(egl.dpy, &list));
        /* the last pair is a colour size: its value is the int before EGL_NONE */
        assert_eq!(run(&[EGL_RED_SIZE, 1, EGL_NONE]).0, [true, false, false, false]);
    }

    #[test]
    fn choose_config_respects_config_size_and_counts_with_null_configs() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let list = [EGL_ALPHA_SIZE, 8, EGL_NONE];
        let mut n: EGLint = -1;
        // SAFETY: EGL_NONE-terminated list; the array has room for config_size entries.
        unsafe {
            let mut out = [core::ptr::null_mut(); 3];
            assert_eq!(eglChooseConfig(egl.dpy, list.as_ptr(), out.as_mut_ptr(), 1, &mut n), TRUE);
            assert_eq!((n, out), (1, [CONFIG_1X, core::ptr::null_mut(), core::ptr::null_mut()]));

            let mut out = [core::ptr::null_mut(); 3];
            assert_eq!(eglChooseConfig(egl.dpy, list.as_ptr(), out.as_mut_ptr(), 0, &mut n), TRUE);
            assert_eq!((n, out[0]), (0, core::ptr::null_mut()));

            assert_eq!(eglChooseConfig(egl.dpy, list.as_ptr(), core::ptr::null_mut(), 0, &mut n), TRUE);
            assert_eq!(n as usize, offered(&[1, 2]).len(), "a null array counts the matches");
        }
    }

    #[test]
    fn choose_config_errors() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let mut n: EGLint = -1;
        let mut out = [core::ptr::null_mut(); 3];
        // SAFETY: null lists are allowed; `n` is writable.
        unsafe {
            assert_eq!(eglChooseConfig(egl.dpy, core::ptr::null(), out.as_mut_ptr(), 3, core::ptr::null_mut()), FALSE);
            assert_eq!(egl_error(), EGL_BAD_PARAMETER);
            assert_eq!(eglChooseConfig(0x1 as EGLDisplay, core::ptr::null(), out.as_mut_ptr(), 3, &mut n), FALSE);
            assert_eq!(egl_error(), EGL_BAD_DISPLAY);
            eglTerminate(egl.dpy);
            assert_eq!(eglChooseConfig(egl.dpy, core::ptr::null(), out.as_mut_ptr(), 3, &mut n), FALSE);
            assert_eq!(egl_error(), EGL_NOT_INITIALIZED);
        }
        assert_eq!(n, -1);
        assert!(out[0].is_null());
    }

    /* ---- pbuffer surfaces --------------------------------------------------------------------------- */

    #[test]
    fn pbuffer_allocates_rgba8888_render_pool_memory() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let s = egl.pbuffer(CONFIG_1X, 16, 4);
        assert!(!s.is_null());
        assert_eq!(egl_error(), EGL_SUCCESS);
        let p = surf(s);
        assert!(!p.window);
        assert_eq!((p.width, p.height, p.format, p.samples), (16, 4, GLES_SURF_RGBA8888, 1));
        assert_eq!(gl.mem_events(), [MemEvent::Alloc { heap: HEAP_DATA, size: 16 * 4 * 4, gpu: p.mem.gpu }]);
        assert_eq!(query_surface(egl.dpy, s, EGL_WIDTH), Ok(16));
        assert_eq!(query_surface(egl.dpy, s, EGL_HEIGHT), Ok(4));
    }

    #[test]
    fn pbuffer_size_limits() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        assert!(!egl.pbuffer(CONFIG_1X, 4096, 1).is_null(), "EGL_MAX_PBUFFER_WIDTH");
        assert!(!egl.pbuffer(CONFIG_1X, 1, 4096).is_null(), "EGL_MAX_PBUFFER_HEIGHT");
        for (w, h) in [(0, 4), (4, 0), (4097, 4), (4, 4097)] {
            assert!(egl.pbuffer(CONFIG_1X, w, h).is_null(), "{w}x{h}");
            assert_eq!(egl_error(), EGL_BAD_ATTRIBUTE, "{w}x{h}");
        }
        /* EGL 1.5 3.5.2 names this one: a negative size is a parameter
         * error, not an attribute error */
        for (w, h) in [(-1, 4), (4, -1), (-1, -1)] {
            assert!(egl.pbuffer(CONFIG_1X, w, h).is_null(), "{w}x{h}");
            assert_eq!(egl_error(), EGL_BAD_PARAMETER, "{w}x{h}");
        }
        assert_eq!(gl.live_allocs(), 3);
    }

    #[test]
    fn a_pbuffer_has_the_colour_format_of_its_config() {
        /* The surface a config describes is the surface the caller gets. A
         * pbuffer used to be RGBA8888 whatever config it was made with, so
         * one on the 565 config was allocated, rendered and read back at 4
         * bytes a pixel with an alpha channel the format does not have --
         * which dEQP-EGL's color_clears saw as the colour channels off by
         * 565's quantisation and alpha off by anything at all. */
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        for (config, format, bpp) in [
            (CONFIG_1X, GLES_SURF_RGBA8888, 4),
            (CONFIG_4X_OR_1X, GLES_SURF_RGBA8888, 4),
            (CONFIG_565, GLES_SURF_RGB565, 2),
        ] {
            gl.mem_events();
            let s = egl.pbuffer(config, 16, 4);
            assert!(!s.is_null(), "config {}", config_id(config));
            let p = surf(s);
            assert_eq!(p.format, format, "config {}", config_id(config));
            assert_eq!(
                gl.mem_events(),
                [MemEvent::Alloc { heap: HEAP_DATA, size: 16 * 4 * bpp, gpu: p.mem.gpu }],
                "config {} allocates its own format",
                config_id(config)
            );
            /* and the surface reports the config it was made with */
            assert_eq!(query_surface(egl.dpy, s, EGL_CONFIG_ID), Ok(config_id(config) as EGLint));
        }
    }

    #[test]
    fn pbuffer_attributes_outside_their_sets_are_refused() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let create = |attrs: &[EGLint]| {
            // SAFETY: the lists below are EGL_NONE-terminated, except the
            // deliberate one-element list, which the walk stops at.
            unsafe { eglCreatePbufferSurface(egl.dpy, CONFIG_1X, attrs.as_ptr()) }
        };
        let size = [EGL_WIDTH, 8, EGL_HEIGHT, 8];
        let case = |extra: &[EGLint], err: EGLint, why: &str| {
            let mut attrs = Vec::from(&size[..]);
            attrs.extend_from_slice(extra);
            attrs.push(EGL_NONE);
            assert!(create(&attrs).is_null(), "{why}");
            assert_eq!(egl_error(), err, "{why}");
        };
        /* an unknown attribute, and one whose list ends without EGL_NONE
         * (dEQP passes {0xffffffff}): the name is judged before the value */
        case(&[0x1234, 0], EGL_BAD_ATTRIBUTE, "an unknown attribute");
        assert!(create(&[EGL_DONT_CARE]).is_null(), "a list of one bad name");
        assert_eq!(egl_error(), EGL_BAD_ATTRIBUTE);
        /* no render-to-texture pbuffers: EGL_BIND_TO_TEXTURE_RGB(A) is
         * EGL_FALSE on every config here */
        case(&[EGL_TEXTURE_FORMAT, EGL_TEXTURE_RGBA, EGL_TEXTURE_TARGET, EGL_TEXTURE_2D], EGL_BAD_MATCH, "a texture");
        /* EGL 1.5 3.5.1: both are EGL_NO_TEXTURE, or neither is */
        case(&[EGL_TEXTURE_TARGET, EGL_TEXTURE_2D], EGL_BAD_MATCH, "a target without a format");
        case(&[EGL_TEXTURE_FORMAT, EGL_TEXTURE_RGB], EGL_BAD_MATCH, "a format without a target");
        case(&[EGL_GL_COLORSPACE, EGL_GL_COLORSPACE_SRGB], EGL_BAD_MATCH, "no sRGB encoding");
        case(&[EGL_VG_ALPHA_FORMAT, EGL_VG_ALPHA_FORMAT_PRE], EGL_BAD_MATCH, "no OpenVG");
        case(&[EGL_LARGEST_PBUFFER, 7], EGL_BAD_ATTRIBUTE, "a boolean that is not one");
        case(&[EGL_TEXTURE_FORMAT, 0x1234], EGL_BAD_ATTRIBUTE, "a format that is no format");

        /* what the surfaces do take */
        let ok = [
            EGL_WIDTH,
            8,
            EGL_HEIGHT,
            8,
            EGL_LARGEST_PBUFFER,
            EGL_FALSE,
            EGL_MIPMAP_TEXTURE,
            EGL_FALSE,
            EGL_TEXTURE_FORMAT,
            EGL_NO_TEXTURE,
            EGL_TEXTURE_TARGET,
            EGL_NO_TEXTURE,
            EGL_GL_COLORSPACE,
            EGL_GL_COLORSPACE_LINEAR,
            EGL_VG_COLORSPACE,
            EGL_VG_COLORSPACE_sRGB,
            EGL_VG_ALPHA_FORMAT,
            EGL_VG_ALPHA_FORMAT_NONPRE,
            EGL_NONE,
        ];
        assert!(!create(&ok).is_null());
        assert_eq!(egl_error(), EGL_SUCCESS);
    }

    #[test]
    fn pbuffer_takes_the_last_value_of_a_repeated_attribute() {
        /* the attribute list is a map (dEQP's choose_config.random repeats
         * names and expects the later value to win) */
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let attrs = [EGL_WIDTH, 4, EGL_HEIGHT, 4, EGL_WIDTH, 16, EGL_NONE];
        // SAFETY: an EGL_NONE-terminated list.
        let s = unsafe { eglCreatePbufferSurface(egl.dpy, CONFIG_1X, attrs.as_ptr()) };
        assert!(!s.is_null());
        assert_eq!(query_surface(egl.dpy, s, EGL_WIDTH), Ok(16));
    }

    #[test]
    fn pixmap_surfaces_report_a_bad_config_before_the_kind_they_cannot_be() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        // SAFETY: plain EGL calls; no pixmap is dereferenced.
        unsafe {
            assert!(eglCreatePixmapSurface(egl.dpy, CONFIG_1X, core::ptr::null_mut(), core::ptr::null()).is_null());
            assert_eq!(egl_error(), EGL_BAD_MATCH, "EGL_PIXMAP_BIT is in no config");
            assert!(
                eglCreatePixmapSurface(egl.dpy, 0x10 as EGLConfig, core::ptr::null_mut(), core::ptr::null()).is_null()
            );
            assert_eq!(egl_error(), EGL_BAD_CONFIG, "a config that is no config comes first");
        }
    }

    #[test]
    fn pbuffer_without_a_size_is_bad_attribute() {
        /* the EGL default size of 0 x 0 has nothing to render into */
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        // SAFETY: null and EGL_NONE-terminated lists.
        unsafe {
            assert!(eglCreatePbufferSurface(egl.dpy, CONFIG_1X, core::ptr::null()).is_null());
            assert_eq!(egl_error(), EGL_BAD_ATTRIBUTE);
            assert!(eglCreatePbufferSurface(egl.dpy, CONFIG_1X, [EGL_WIDTH, 8, EGL_NONE].as_ptr()).is_null());
            assert_eq!(egl_error(), EGL_BAD_ATTRIBUTE);
        }
    }

    #[test]
    fn pbuffer_errors() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        assert!(egl.pbuffer(core::ptr::null_mut(), 4, 4).is_null());
        assert_eq!(egl_error(), EGL_BAD_CONFIG);
        let attrs = [EGL_WIDTH, 4, EGL_HEIGHT, 4, EGL_NONE];
        // SAFETY: an EGL_NONE-terminated list.
        assert!(unsafe { eglCreatePbufferSurface(0x1 as EGLDisplay, CONFIG_1X, attrs.as_ptr()) }.is_null());
        assert_eq!(egl_error(), EGL_BAD_DISPLAY);
        assert!(gl.mem_events().is_empty());
    }

    #[test]
    fn pbuffer_allocation_failure_is_bad_alloc_and_takes_no_slot() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        gl.fail_alloc_in(1);
        assert!(egl.pbuffer(CONFIG_1X, 4, 4).is_null());
        assert_eq!(egl_error(), EGL_BAD_ALLOC);
        for _ in 0..EGL_MAX_SURFACES {
            assert!(!egl.pbuffer(CONFIG_1X, 4, 4).is_null());
        }
    }

    #[test]
    fn surface_pool_holds_four() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let s: Vec<EGLSurface> = (0..EGL_MAX_SURFACES).map(|_| egl.pbuffer(CONFIG_1X, 4, 4)).collect();
        assert!(s.iter().all(|s| !s.is_null()));
        assert!(s.iter().enumerate().all(|(i, a)| s[..i].iter().all(|b| a != b)), "distinct handles");
        assert!(egl.pbuffer(CONFIG_1X, 4, 4).is_null());
        assert_eq!(egl_error(), EGL_BAD_ALLOC);
        let mut win = window(GLES_NATIVE_FORMAT_RGBA8888, 4, 4);
        assert!(window_surface(egl.dpy, CONFIG_1X, &mut win).is_null(), "windows share the pool");
        assert_eq!(egl_error(), EGL_BAD_ALLOC);
        assert_eq!(gl.live_allocs(), 1 + EGL_MAX_SURFACES, "a refused surface allocates nothing");
        // SAFETY: plain EGL call.
        assert_eq!(unsafe { eglDestroySurface(egl.dpy, s[1]) }, TRUE);
        assert!(!egl.pbuffer(CONFIG_1X, 4, 4).is_null());
    }

    #[test]
    fn destroy_surface_frees_the_pbuffer_and_kills_the_handle() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let s = egl.pbuffer(CONFIG_1X, 4, 4);
        let gpu = surf(s).mem.gpu;
        gl.mem_events();
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglDestroySurface(egl.dpy, s), TRUE);
            assert_eq!(gl.mem_events(), [MemEvent::Free { gpu }]);
            assert_eq!(eglDestroySurface(egl.dpy, s), FALSE);
            assert_eq!(egl_error(), EGL_BAD_SURFACE);
            assert_eq!(eglDestroySurface(egl.dpy, NO_SURFACE), FALSE);
            assert_eq!(egl_error(), EGL_BAD_SURFACE);
            assert_eq!(eglDestroySurface(0x1 as EGLDisplay, s), FALSE);
            assert_eq!(egl_error(), EGL_BAD_DISPLAY);
        }
        assert_eq!(query_surface(egl.dpy, s, EGL_WIDTH), Err(EGL_BAD_SURFACE));
    }

    /// A destroyed surface's own depth/stencil (`GlesDraw::depth_id`: bit 31
    /// and its colour buffer's page) is gone for the backend.
    #[test]
    fn destroying_a_surface_tells_the_backend_its_depth_stencil_is_gone() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let s = egl.pbuffer(CONFIG_1X, 4, 4);
        let id = 0x8000_0000 | (surf(s).mem.gpu >> 12) as u32;
        gl.gones();
        // SAFETY: plain EGL call.
        assert_eq!(unsafe { eglDestroySurface(egl.dpy, s) }, TRUE);
        assert_eq!(gl.gones(), [id]);
    }

    #[test]
    fn destroying_the_current_surface_releases_the_context() {
        /* this EGL does not defer the destruction: the context is unbound first */
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (s, c) = egl.current(4, 4);
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglDestroySurface(egl.dpy, s), TRUE);
            assert_eq!(eglGetCurrentContext(), NO_CONTEXT);
            assert_eq!(eglGetCurrentSurface(EGL_DRAW), NO_SURFACE);
            assert!(context::gl_ctx.is_null());
        }
        assert_eq!((gl.flushes(), gl.finishes()), (1, 1), "the frame is completed before the surface goes");
        /* the context survives and takes another surface */
        let s2 = egl.pbuffer(CONFIG_1X, 2, 2);
        assert_eq!(egl.make_current(s2, c), TRUE);
    }

    #[test]
    fn query_surface_fixed_attributes() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let s = egl.pbuffer(CONFIG_1X, 4, 4);
        let q = |a| query_surface(egl.dpy, s, a);
        assert_eq!(q(EGL_CONFIG_ID), Ok(1));
        /* a pbuffer renders to the back buffer, EGL 1.5 table 3.5, and its
         * contents survive a swap (there is one buffer, rendered in place) */
        assert_eq!(q(EGL_RENDER_BUFFER), Ok(EGL_BACK_BUFFER));
        assert_eq!(q(EGL_SWAP_BEHAVIOR), Ok(EGL_BUFFER_PRESERVED));
        assert_eq!(q(EGL_MULTISAMPLE_RESOLVE), Ok(EGL_MULTISAMPLE_RESOLVE_DEFAULT));
        assert_eq!(q(EGL_HORIZONTAL_RESOLUTION), Ok(EGL_UNKNOWN));
        assert_eq!(q(EGL_VERTICAL_RESOLUTION), Ok(EGL_UNKNOWN));
        assert_eq!(q(EGL_PIXEL_ASPECT_RATIO), Ok(EGL_UNKNOWN));
        assert_eq!(q(EGL_LARGEST_PBUFFER), Ok(EGL_FALSE));
        assert_eq!(q(EGL_TEXTURE_FORMAT), Ok(EGL_NO_TEXTURE));
        assert_eq!(q(EGL_TEXTURE_TARGET), Ok(EGL_NO_TEXTURE));
        assert_eq!(q(EGL_MIPMAP_TEXTURE), Ok(EGL_FALSE));
        assert_eq!(q(EGL_MIPMAP_LEVEL), Ok(0));
        assert_eq!(q(EGL_VG_ALPHA_FORMAT), Ok(EGL_VG_ALPHA_FORMAT_NONPRE));
        assert_eq!(q(EGL_VG_COLORSPACE), Ok(EGL_VG_COLORSPACE_sRGB));
    }

    #[cfg(feature = "gpu-powervr")] /* the GC7000 build has no 4x config */
    #[test]
    fn query_surface_config_id_of_a_multisampled_surface() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let s = egl.pbuffer(CONFIG_4X, 4, 4);
        assert_eq!(surf(s).samples, 4);
        assert_eq!(query_surface(egl.dpy, s, EGL_CONFIG_ID), Ok(2));
    }

    #[test]
    fn query_surface_config_id_of_an_rgb565_surface() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let mut win = window(GLES_NATIVE_FORMAT_RGB565, 8, 4);
        let s = window_surface(egl.dpy, CONFIG_565, &mut win);
        assert!(!s.is_null());
        assert_eq!(query_surface(egl.dpy, s, EGL_CONFIG_ID), Ok(3));
    }

    #[test]
    fn query_surface_errors() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let s = egl.pbuffer(CONFIG_1X, 4, 4);
        assert_eq!(query_surface(egl.dpy, s, EGL_RED_SIZE), Err(EGL_BAD_ATTRIBUTE));
        assert_eq!(query_surface(egl.dpy, 0x10 as EGLSurface, EGL_WIDTH), Err(EGL_BAD_SURFACE));
        assert_eq!(query_surface(0x1 as EGLDisplay, s, EGL_WIDTH), Err(EGL_BAD_DISPLAY));
        // SAFETY: null is rejected.
        assert_eq!(unsafe { eglQuerySurface(egl.dpy, s, EGL_WIDTH, core::ptr::null_mut()) }, FALSE);
        assert_eq!(egl_error(), EGL_BAD_PARAMETER);
    }

    #[test]
    fn surface_attrib_accepts_what_the_surfaces_already_do() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let s = egl.pbuffer(CONFIG_1X, 4, 4);
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglSurfaceAttrib(egl.dpy, s, EGL_SWAP_BEHAVIOR, EGL_BUFFER_PRESERVED), TRUE);
            assert_eq!(eglSurfaceAttrib(egl.dpy, s, EGL_MIPMAP_LEVEL, 3), TRUE);
            assert_eq!(eglSurfaceAttrib(egl.dpy, s, EGL_MULTISAMPLE_RESOLVE, EGL_MULTISAMPLE_RESOLVE_DEFAULT), TRUE);
            assert_eq!(egl_error(), EGL_SUCCESS);
            /* both swap behaviours are legal -- the config carries
             * EGL_SWAP_BEHAVIOR_PRESERVED_BIT -- and the query answers with
             * the one that was set */
            assert_eq!(eglSurfaceAttrib(egl.dpy, s, EGL_SWAP_BEHAVIOR, EGL_BUFFER_DESTROYED), TRUE);
            assert_eq!(query_surface(egl.dpy, s, EGL_SWAP_BEHAVIOR), Ok(EGL_BUFFER_DESTROYED));
            assert_eq!(eglSurfaceAttrib(egl.dpy, s, EGL_SWAP_BEHAVIOR, EGL_BUFFER_PRESERVED), TRUE);
            assert_eq!(query_surface(egl.dpy, s, EGL_SWAP_BEHAVIOR), Ok(EGL_BUFFER_PRESERVED));
            /* no box filter: EGL_MULTISAMPLE_RESOLVE_BOX_BIT is in no config */
            assert_eq!(eglSurfaceAttrib(egl.dpy, s, EGL_MULTISAMPLE_RESOLVE, EGL_MULTISAMPLE_RESOLVE_BOX), FALSE);
            assert_eq!(egl_error(), EGL_BAD_MATCH);
            assert_eq!(eglSurfaceAttrib(egl.dpy, s, EGL_WIDTH, 8), FALSE);
            assert_eq!(egl_error(), EGL_BAD_ATTRIBUTE);
            assert_eq!(eglSurfaceAttrib(egl.dpy, NO_SURFACE, EGL_MIPMAP_LEVEL, 0), FALSE);
            assert_eq!(egl_error(), EGL_BAD_SURFACE);
            assert_eq!(eglSurfaceAttrib(0x1 as EGLDisplay, s, EGL_MIPMAP_LEVEL, 0), FALSE);
            assert_eq!(egl_error(), EGL_BAD_DISPLAY);
        }
    }

    #[test]
    fn unsupported_surface_kinds_and_operations() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let s = egl.pbuffer(CONFIG_1X, 4, 4);
        gl.mem_events();
        // SAFETY: plain EGL calls; the native handles are never dereferenced.
        unsafe {
            /* no texture-bindable configs */
            assert_eq!(eglBindTexImage(egl.dpy, s, EGL_BACK_BUFFER), FALSE);
            assert_eq!(egl_error(), EGL_BAD_MATCH);
            assert_eq!(eglReleaseTexImage(egl.dpy, s, EGL_BACK_BUFFER), FALSE);
            assert_eq!(egl_error(), EGL_BAD_MATCH);
            assert_eq!(eglBindTexImage(egl.dpy, NO_SURFACE, EGL_BACK_BUFFER), FALSE);
            assert_eq!(egl_error(), EGL_BAD_SURFACE);
            /* no pixmaps */
            assert_eq!(eglCopyBuffers(egl.dpy, s, 0x1000 as EGLNativePixmapType), FALSE);
            assert_eq!(egl_error(), EGL_BAD_NATIVE_PIXMAP);
            assert_eq!(eglCopyBuffers(egl.dpy, NO_SURFACE, 0x1000 as EGLNativePixmapType), FALSE);
            assert_eq!(egl_error(), EGL_BAD_SURFACE);
            assert!(
                eglCreatePixmapSurface(egl.dpy, CONFIG_1X, 0x1000 as EGLNativePixmapType, core::ptr::null()).is_null()
            );
            assert_eq!(egl_error(), EGL_BAD_MATCH);
            assert!(
                eglCreatePlatformPixmapSurface(egl.dpy, CONFIG_1X, 0x1000 as *mut c_void, core::ptr::null()).is_null()
            );
            assert_eq!(egl_error(), EGL_BAD_MATCH);
            /* no client buffers */
            let buf = 0x1000 as EGLClientBuffer;
            assert!(eglCreatePbufferFromClientBuffer(
                egl.dpy,
                EGL_OPENVG_IMAGE as EGLenum,
                buf,
                CONFIG_1X,
                core::ptr::null()
            )
            .is_null());
            assert_eq!(egl_error(), EGL_BAD_PARAMETER);
            assert!(eglCreatePixmapSurface(0x1 as EGLDisplay, CONFIG_1X, core::ptr::null_mut(), core::ptr::null())
                .is_null());
            assert_eq!(egl_error(), EGL_BAD_DISPLAY);
        }
        assert!(gl.mem_events().is_empty());
        assert_eq!(state().surf.iter().filter(|s| s.used).count(), 1);
    }

    /* ---- window surfaces ------------------------------------------------------------------------------- */

    #[test]
    fn window_surface_maps_the_window_at_a_render_pool_va_for_each_format() {
        /* the mock platform has mmu_map only: a VA range from the pool, repointed at the window's pages */
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        for (native, config, format, bytes) in [
            (GLES_NATIVE_FORMAT_RGBA8888, CONFIG_1X, GLES_SURF_RGBA8888, 8 * 4 * 4),
            (GLES_NATIVE_FORMAT_XRGB8888, CONFIG_4X_OR_1X, GLES_SURF_XRGB8888, 8 * 4 * 4),
            (GLES_NATIVE_FORMAT_RGB565, CONFIG_565, GLES_SURF_RGB565, 8 * 4 * 2),
        ] {
            gl.mem_events();
            let mut win = window(native, 8, 4);
            let s = window_surface(egl.dpy, config, &mut win);
            assert!(!s.is_null(), "format {native}: 0x{:x}", egl_error());
            let p = surf(s);
            assert!(p.window);
            assert_eq!((p.width, p.height, p.format, p.samples), (8, 4, format, config_samples(config)));
            assert_eq!(gl.mem_events(), [MemEvent::Alloc { heap: HEAP_DATA, size: bytes, gpu: p.mem.gpu }]);
            assert_eq!(p.mem.phys, win.phys);
            assert!(p.mem.cpu.is_null(), "the pool pages behind the VA are not the window");
            assert_eq!(query_surface(egl.dpy, s, EGL_WIDTH), Ok(8));
            assert_eq!(query_surface(egl.dpy, s, EGL_HEIGHT), Ok(4));
        }
        assert_eq!(egl_error(), EGL_SUCCESS);
    }

    #[test]
    fn platform_window_surface_is_the_same_path() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let mut win = window(GLES_NATIVE_FORMAT_XRGB8888, 6, 2);
        // SAFETY: `win` is a valid native window description.
        let s = unsafe {
            eglCreatePlatformWindowSurface(
                egl.dpy,
                CONFIG_1X,
                &mut win as *mut GlesNativeWindow as *mut c_void,
                core::ptr::null(),
            )
        };
        assert!(!s.is_null());
        let p = surf(s);
        assert!(p.window);
        assert_eq!((p.width, p.height, p.format), (6, 2, GLES_SURF_XRGB8888));
    }

    #[test]
    fn window_surface_invalid_windows_are_bad_native_window() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let ok = window(GLES_NATIVE_FORMAT_RGBA8888, 8, 4);
        // SAFETY: null is rejected.
        assert!(
            unsafe { eglCreateWindowSurface(egl.dpy, CONFIG_1X, core::ptr::null_mut(), core::ptr::null()) }.is_null()
        );
        assert_eq!(egl_error(), EGL_BAD_NATIVE_WINDOW);
        for mut bad in [
            GlesNativeWindow { phys: 0, ..ok },
            GlesNativeWindow { width: 0, stride_bytes: 0, ..ok },
            GlesNativeWindow { height: 0, ..ok },
            GlesNativeWindow { format: 0, ..ok },
            GlesNativeWindow { format: 7, ..ok },
            GlesNativeWindow { stride_bytes: 8 * 4 + 4, ..ok },
        ] {
            assert!(window_surface(egl.dpy, CONFIG_1X, &mut bad).is_null(), "{bad:?}");
            assert_eq!(egl_error(), EGL_BAD_NATIVE_WINDOW, "{bad:?}");
        }
        assert!(gl.mem_events().is_empty());
        assert!(state().surf.iter().all(|s| !s.used));
    }

    #[test]
    fn window_surface_config_depth_must_match_the_window() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let mut w565 = window(GLES_NATIVE_FORMAT_RGB565, 8, 4);
        let mut w8888 = window(GLES_NATIVE_FORMAT_XRGB8888, 8, 4);
        assert!(window_surface(egl.dpy, CONFIG_1X, &mut w565).is_null());
        assert_eq!(egl_error(), EGL_BAD_MATCH);
        assert!(window_surface(egl.dpy, CONFIG_4X_OR_1X, &mut w565).is_null());
        assert_eq!(egl_error(), EGL_BAD_MATCH);
        assert!(window_surface(egl.dpy, CONFIG_565, &mut w8888).is_null());
        assert_eq!(egl_error(), EGL_BAD_MATCH);
        assert!(gl.mem_events().is_empty());
    }

    #[test]
    fn window_surface_bad_config_display_and_state() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let mut win = window(GLES_NATIVE_FORMAT_RGBA8888, 8, 4);
        assert!(window_surface(egl.dpy, core::ptr::null_mut(), &mut win).is_null());
        assert_eq!(egl_error(), EGL_BAD_CONFIG);
        assert!(window_surface(0x1 as EGLDisplay, CONFIG_1X, &mut win).is_null());
        assert_eq!(egl_error(), EGL_BAD_DISPLAY);
        // SAFETY: plain EGL call.
        unsafe { eglTerminate(egl.dpy) };
        assert!(window_surface(egl.dpy, CONFIG_1X, &mut win).is_null());
        assert_eq!(egl_error(), EGL_NOT_INITIALIZED);
    }

    #[test]
    fn window_surface_allocation_failure_is_bad_alloc_and_takes_no_slot() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let mut win = window(GLES_NATIVE_FORMAT_RGBA8888, 8, 4);
        gl.fail_alloc_in(1);
        assert!(window_surface(egl.dpy, CONFIG_1X, &mut win).is_null());
        assert_eq!(egl_error(), EGL_BAD_ALLOC);
        assert!(state().surf.iter().all(|s| !s.used));
    }

    #[test]
    fn destroying_a_window_surface_keeps_its_va_range() {
        /* known quirk (see destroy_surface): the pool VA range a window was
         * mapped at is not reclaimable without an unmap, so it is left */
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let mut win = window(GLES_NATIVE_FORMAT_RGBA8888, 8, 4);
        let s = window_surface(egl.dpy, CONFIG_1X, &mut win);
        gl.mem_events();
        // SAFETY: plain EGL call.
        assert_eq!(unsafe { eglDestroySurface(egl.dpy, s) }, TRUE);
        assert!(gl.mem_events().is_empty());
        assert!(state().surf.iter().all(|s| !s.used), "the slot itself is free again");
    }

    #[test]
    fn window_surface_without_any_mapping_hook_is_bad_native_window() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let _ops = OpsSwap::new(|o| o.mmu_map = None);
        let mut win = window(GLES_NATIVE_FORMAT_RGBA8888, 8, 4);
        assert!(window_surface(egl.dpy, CONFIG_1X, &mut win).is_null());
        assert_eq!(egl_error(), EGL_BAD_NATIVE_WINDOW);
        assert!(gl.mem_events().is_empty());
    }

    #[test]
    fn window_surface_prefers_map_phys_and_uses_no_pool_memory() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let _ops = OpsSwap::new(|o| o.map_phys = Some(t_map_phys));
        let mut pixels = std::vec![0u8; 8 * 4 * 2];
        let mut win = window(GLES_NATIVE_FORMAT_RGB565, 8, 4);
        win.phys = pixels.as_mut_ptr() as u64;
        let s = window_surface(egl.dpy, CONFIG_565, &mut win);
        assert!(!s.is_null());
        let p = surf(s);
        assert_eq!((p.mem.phys, p.mem.size), (win.phys, 8 * 4 * 2), "height x stride bytes are mapped");
        assert_eq!(p.mem.cpu as u64, win.phys);
        assert!(gl.mem_events().is_empty());
    }

    #[test]
    fn window_surface_map_phys_failure_is_bad_native_window() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let _ops = OpsSwap::new(|o| o.map_phys = Some(t_map_phys));
        MAP_PHYS_FAILS.store(true, Ordering::Relaxed);
        let mut win = window(GLES_NATIVE_FORMAT_RGBA8888, 8, 4);
        assert!(window_surface(egl.dpy, CONFIG_1X, &mut win).is_null());
        assert_eq!(egl_error(), EGL_BAD_NATIVE_WINDOW);
        assert!(state().surf.iter().all(|s| !s.used));
    }

    #[test]
    fn a_context_on_a_window_renders_into_it_in_the_windows_format() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let mut win = window(GLES_NATIVE_FORMAT_XRGB8888, 16, 8);
        let s = window_surface(egl.dpy, CONFIG_4X_OR_1X, &mut win);
        let c = egl.context(CONFIG_4X_OR_1X);
        assert_eq!(egl.make_current(s, c), TRUE);
        let glc = context::current().expect("a current GL context");
        assert_eq!(glc.default_surface.gpu, surf(s).mem.gpu);
        assert!(glc.default_surface_valid && !glc.default_surface_owned);
        assert_eq!((glc.target_w, glc.target_h), (16, 8));
        assert_eq!((glc.default_format, glc.default_samples), (GLES_SURF_XRGB8888, config_samples(CONFIG_4X_OR_1X)));
        assert_eq!(glc.viewport, [0, 0, 16, 8]);
    }

    #[test]
    fn a_window_is_drawn_top_down_and_a_pbuffer_y_up_as_an_fbo_is() {
        /* one context moved between the two: the flag follows the surface */
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let mut win = window(GLES_NATIVE_FORMAT_XRGB8888, 16, 8);
        let w = window_surface(egl.dpy, CONFIG_1X, &mut win);
        let p = egl.pbuffer(CONFIG_1X, 16, 8);
        let c = egl.context(CONFIG_1X);
        let flip = || framebuffer::fb_flip_y(context::current().expect("a current GL context"));
        assert_eq!(egl.make_current(p, c), TRUE);
        assert!(!flip(), "first use on a pbuffer: stored bottom-up");
        assert_eq!(egl.make_current(w, c), TRUE);
        assert!(flip(), "a window is scanned out top-down");
        assert_eq!(egl.make_current(p, c), TRUE);
        assert!(!flip(), "back on the pbuffer");
        /* and the draw carries it */
        gl.clear_recordings();
        // SAFETY: plain GL calls on the current context.
        unsafe {
            glClear(GL_COLOR_BUFFER_BIT);
            glFinish();
        }
        let draws = gl.draws();
        assert_eq!(draws.len(), 1);
        assert!(!draws[0].draw.flip_y && draws[0].draw.target_gpu == surf(p).mem.gpu);
    }

    #[test]
    fn read_pixels_reads_a_pbuffer_bottom_up_and_a_window_top_down() {
        /* memory row r holds the grey level 10 (r + 1) in every byte, so the
         * GL row it reads back as names the orientation in any channel order */
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let mut win = window(GLES_NATIVE_FORMAT_XRGB8888, 4, 4);
        let w = window_surface(egl.dpy, CONFIG_1X, &mut win);
        let p = egl.pbuffer(CONFIG_1X, 4, 4);
        let c = egl.context(CONFIG_1X);
        let first_byte_of_gl_row = |s: EGLSurface| -> [u8; 4] {
            assert_eq!(egl.make_current(s, c), TRUE);
            let mut rows = std::vec![0u8; 4 * 4 * 4];
            for (r, row) in rows.chunks_mut(16).enumerate() {
                row.fill(10 * (r as u8 + 1));
            }
            gl.gpu_render(surf(s).mem.gpu, &rows);
            let mut out = [0u8; 4 * 4 * 4];
            // SAFETY: `out` holds 4 rows of 4 RGBA pixels.
            unsafe { glReadPixels(0, 0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, out.as_mut_ptr() as *mut c_void) };
            core::array::from_fn(|y| out[y * 16])
        };
        assert_eq!(first_byte_of_gl_row(p), [10, 20, 30, 40], "a pbuffer: GL row y is memory row y");
        assert_eq!(first_byte_of_gl_row(w), [40, 30, 20, 10], "a window: GL row 0 is the last memory row");
    }

    /// A window that is a view into a wider scan-out buffer: an even pitch
    /// of whole pixels, at least tex_pitch(width), at most MAX_NATIVE_SIDE;
    /// a pixmap (EGLImage) keeps the packed rule.
    #[test]
    fn a_window_may_be_a_view_with_a_wider_even_pitch() {
        let ok = window(GLES_NATIVE_FORMAT_XRGB8888, 4, 3);
        assert_eq!(native_window_pitch(&ok), Some((GLES_SURF_XRGB8888, 4)));
        let view = GlesNativeWindow { stride_bytes: 8 * 4, ..ok };
        assert_eq!(native_window_pitch(&view), Some((GLES_SURF_XRGB8888, 8)));
        assert_eq!(native_buffer_format(&view), None, "a pixmap stays packed");
        for (stride, why) in [(9 * 4, "an odd pitch"), (8 * 4 + 2, "not whole pixels"), (2 * 4, "below the width")] {
            assert_eq!(native_window_pitch(&GlesNativeWindow { stride_bytes: stride, ..ok }), None, "{why}");
        }
        let side = MAX_NATIVE_SIDE;
        assert_eq!(
            native_window_pitch(&GlesNativeWindow { stride_bytes: side * 4, ..ok }),
            Some((GLES_SURF_XRGB8888, side))
        );
        assert_eq!(
            native_window_pitch(&GlesNativeWindow { stride_bytes: (side + 2) * 4, ..ok }),
            None,
            "past the side"
        );
        let odd = window(GLES_NATIVE_FORMAT_RGB565, 7, 3);
        assert_eq!(
            native_window_pitch(&GlesNativeWindow { stride_bytes: 10 * 2, ..odd }),
            Some((GLES_SURF_RGB565, 10))
        );
        assert_eq!(native_window_pitch(&odd), None, "7 packed has no pad pixel");
    }

    /// A 4x3 window that is a view at a pitch of 8 pixels: the draw carries
    /// the pitch to the backend, and glReadPixels reads the window's pixels
    /// at that pitch -- its first and last rows, and never the padding.
    #[test]
    fn a_view_window_is_drawn_and_read_at_its_pitch() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (w, h, pitch) = (4u32, 3u32, 8u32);
        let mut win = GlesNativeWindow { stride_bytes: pitch * 4, ..window(GLES_NATIVE_FORMAT_RGBA8888, w, h) };
        let s = window_surface(egl.dpy, CONFIG_1X, &mut win);
        assert!(!s.is_null(), "0x{:x}", egl_error());
        assert_eq!(surf(s).pitch, pitch);
        let c = egl.context(CONFIG_1X);
        assert_eq!(egl.make_current(s, c), TRUE);
        assert_eq!(context::current().unwrap().default_pitch, pitch);

        /* memory: pixel (x, row) = (10 * row + x, 0, 0, 255); the padding 0xEE */
        let mut mem = std::vec![0xEEu8; (pitch * h * 4) as usize];
        for row in 0..h {
            for x in 0..w {
                let o = ((row * pitch + x) * 4) as usize;
                mem[o..o + 4].copy_from_slice(&[(10 * row + x) as u8, 0, 0, 255]);
            }
        }
        gl.gpu_render(surf(s).mem.gpu, &mem);
        let mut out = [0u8; 4 * 3 * 4];
        // SAFETY: `out` holds 3 rows of 4 RGBA pixels.
        unsafe { glReadPixels(0, 0, w as i32, h as i32, GL_RGBA, GL_UNSIGNED_BYTE, out.as_mut_ptr() as *mut c_void) };
        assert_eq!(gl_error(), GL_NO_ERROR);
        /* a window: GL row 0 is the last memory row */
        let red = |gl_row: usize| -> std::vec::Vec<u8> { (0..4).map(|x| out[gl_row * 16 + x * 4]).collect() };
        assert_eq!(red(0), [20, 21, 22, 23], "GL row 0: the last memory row");
        assert_eq!(red(2), [0, 1, 2, 3], "GL row 2: the first memory row");
        assert!(out.iter().all(|&b| b != 0xEE), "no padding byte was read");

        /* a draw into it carries the pitch */
        gl.clear_recordings();
        // SAFETY: plain GL calls on the current context.
        unsafe {
            glClear(GL_COLOR_BUFFER_BIT);
            glFinish();
        }
        let draws = gl.draws();
        assert_eq!(draws.len(), 1);
        assert_eq!((draws[0].draw.target_w, draws[0].draw.target_pitch), (w, pitch));

        /* back on a pbuffer: packed again */
        let p = egl.pbuffer(CONFIG_1X, 4, 4);
        assert_eq!(egl.make_current(p, c), TRUE);
        assert_eq!(context::current().unwrap().default_pitch, 0);
    }

    #[test]
    fn a_window_the_display_scans_bottom_up_is_drawn_and_read_y_up() {
        /* GLES_NATIVE_WINDOW_BOTTOM_UP: the flag is stripped from the format
         * (an XRGB8888 window keeps its 1X config), and the window is stored
         * as a pbuffer is: GL row y is memory row y, the draw not mirrored */
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let mut win = window(GLES_NATIVE_FORMAT_XRGB8888 | GLES_NATIVE_WINDOW_BOTTOM_UP, 4, 4);
        let w = window_surface(egl.dpy, CONFIG_1X, &mut win);
        assert!(!w.is_null(), "error 0x{:x}", egl_error());
        assert_eq!(surf(w).format, GLES_SURF_XRGB8888);
        let c = egl.context(CONFIG_1X);
        assert_eq!(egl.make_current(w, c), TRUE);
        assert!(!framebuffer::fb_flip_y(context::current().expect("a current GL context")));
        let mut rows = std::vec![0u8; 4 * 4 * 4];
        for (r, row) in rows.chunks_mut(16).enumerate() {
            row.fill(10 * (r as u8 + 1));
        }
        gl.gpu_render(surf(w).mem.gpu, &rows);
        let mut out = [0u8; 4 * 4 * 4];
        // SAFETY: `out` holds 4 rows of 4 RGBA pixels.
        unsafe { glReadPixels(0, 0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, out.as_mut_ptr() as *mut c_void) };
        let first: [u8; 4] = core::array::from_fn(|y| out[y * 16]);
        assert_eq!(first, [10, 20, 30, 40], "a bottom-up window: GL row y is memory row y");
        gl.clear_recordings();
        // SAFETY: plain GL calls on the current context.
        unsafe {
            glClear(GL_COLOR_BUFFER_BIT);
            glFinish();
        }
        assert!(gl.draws().iter().all(|d| !d.draw.flip_y), "the draw is not mirrored");
        /* the 565 config check reads the stripped format too */
        let mut w565 = window(GLES_NATIVE_FORMAT_RGB565, 4, 4);
        w565.format |= GLES_NATIVE_WINDOW_BOTTOM_UP;
        assert!(!window_surface(egl.dpy, CONFIG_565, &mut w565).is_null(), "error 0x{:x}", egl_error());
    }

    /* ---- contexts ------------------------------------------------------------------------------------------ */

    #[test]
    fn create_context_accepts_client_versions_2_and_3() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        for version in [2, 3] {
            let attrs = [EGL_CONTEXT_CLIENT_VERSION, version, EGL_NONE];
            // SAFETY: an EGL_NONE-terminated list.
            let c = unsafe { eglCreateContext(egl.dpy, CONFIG_1X, NO_CONTEXT, attrs.as_ptr()) };
            assert!(!c.is_null(), "version {version}");
            /* one kind of context: an ES 2 request gets the ES 3 context too */
            assert_eq!(query_context(egl.dpy, c, EGL_CONTEXT_CLIENT_VERSION), Ok(3));
        }
        assert_eq!(egl_error(), EGL_SUCCESS);
    }

    #[test]
    fn create_context_other_client_versions_are_bad_match() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        for version in [0, 1, 4] {
            let attrs = [EGL_CONTEXT_CLIENT_VERSION, version, EGL_NONE];
            // SAFETY: an EGL_NONE-terminated list.
            assert!(unsafe { eglCreateContext(egl.dpy, CONFIG_1X, NO_CONTEXT, attrs.as_ptr()) }.is_null());
            assert_eq!(egl_error(), EGL_BAD_MATCH, "version {version}");
        }
        /* EGL's default client version is 1: OpenGL ES 1.x is not offered */
        // SAFETY: null and EGL_NONE-terminated lists.
        unsafe {
            assert!(eglCreateContext(egl.dpy, CONFIG_1X, NO_CONTEXT, core::ptr::null()).is_null());
            assert_eq!(egl_error(), EGL_BAD_MATCH);
            assert!(eglCreateContext(egl.dpy, CONFIG_1X, NO_CONTEXT, [EGL_NONE].as_ptr()).is_null());
            assert_eq!(egl_error(), EGL_BAD_MATCH);
        }
        assert!(state().ctx.iter().all(|c| !c.used));
    }

    #[test]
    fn shared_contexts_see_one_anothers_objects() {
        /* ES 3.0 2.6.1: buffers, textures, renderbuffers, shaders and
         * programs belong to the share group, and the bindings do not. */
        let gl = TestGl::new(8, 8);
        let mut egl = Egl::new(&gl);
        egl.free_testgl_context();
        let s = egl.pbuffer(CONFIG_1X, 4, 4);
        let a = egl.context(CONFIG_1X);
        let b = egl.shared_context(CONFIG_1X, a);
        assert!(!b.is_null(), "0x{:x}", egl_error());
        assert_eq!(egl.make_current(s, a), TRUE);
        let tex = texture(8, 2);
        let gpu = context::current().unwrap().objs().textures[tex as usize].mem.gpu;

        assert_eq!(egl.make_current(s, b), TRUE);
        let c = context::current().unwrap();
        assert_eq!(c.objs().textures[tex as usize].mem.gpu, gpu, "the same texture object");
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glIsTexture(tex) }, 1);
        /* the binding is the context's own, though */
        assert_eq!(c.bound_texture_2d[0], 0, "a fresh context binds nothing");
    }

    #[test]
    fn an_unshared_context_sees_none_of_them() {
        let gl = TestGl::new(8, 8);
        let mut egl = Egl::new(&gl);
        egl.free_testgl_context();
        let s = egl.pbuffer(CONFIG_1X, 4, 4);
        let (a, b) = (egl.context(CONFIG_1X), egl.context(CONFIG_1X));
        assert_eq!(egl.make_current(s, a), TRUE);
        let tex = texture(8, 2);
        assert_eq!(egl.make_current(s, b), TRUE);
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glIsTexture(tex) }, 0, "another group's name is nothing here");
        assert!(!context::current().unwrap().objs().textures[tex as usize].used);
    }

    #[test]
    fn a_share_groups_objects_outlive_the_context_that_made_them() {
        let gl = TestGl::new(8, 8);
        let mut egl = Egl::new(&gl);
        egl.free_testgl_context();
        let s = egl.pbuffer(CONFIG_1X, 4, 4);
        let a = egl.context(CONFIG_1X);
        let b = egl.shared_context(CONFIG_1X, a);
        assert_eq!(egl.make_current(s, a), TRUE);
        let tex = texture(8, 2);
        let gpu = context::current().unwrap().objs().textures[tex as usize].mem.gpu;
        let live = gl.live_allocs();

        /* the context that created the texture goes; the group does not */
        assert_eq!(egl.make_current(s, b), TRUE);
        // SAFETY: a live context handle.
        assert_eq!(unsafe { eglDestroyContext(egl.dpy, a) }, TRUE);
        assert_eq!(gl.live_allocs(), live, "the texture's storage stays");
        assert_eq!(context::current().unwrap().objs().textures[tex as usize].mem.gpu, gpu);

        /* and goes with the last member of the group */
        assert_eq!(egl.release(), TRUE);
        // SAFETY: a live context handle.
        assert_eq!(unsafe { eglDestroyContext(egl.dpy, b) }, TRUE);
        assert_eq!(gl.live_allocs(), live - 1, "freed once, with the last of the group");
    }

    #[test]
    fn a_share_context_that_is_no_context_is_bad_context() {
        /* EGL 1.5 3.7.1. It used to be EGL_BAD_MATCH for every share
         * context, valid or not, because there were no share groups. */
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let attrs = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
        // SAFETY: an EGL_NONE-terminated list.
        assert!(unsafe { eglCreateContext(egl.dpy, CONFIG_1X, 0x10 as EGLContext, attrs.as_ptr()) }.is_null());
        assert_eq!(egl_error(), EGL_BAD_CONTEXT);
    }

    #[test]
    fn create_context_attributes_outside_their_sets_are_refused() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let create = |attrs: &[EGLint]| {
            // SAFETY: the lists below are EGL_NONE-terminated, except the
            // deliberate one-element list, which the walk stops at.
            unsafe { eglCreateContext(egl.dpy, CONFIG_1X, NO_CONTEXT, attrs.as_ptr()) }
        };
        let case = |attrs: &[EGLint], err: EGLint, why: &str| {
            assert!(create(attrs).is_null(), "{why}");
            assert_eq!(egl_error(), err, "{why}");
        };
        /* dEQP's two lists: an unknown name, and a name with no value
         * behind it -- the name is judged before the value is read */
        case(&[0, EGL_NONE], EGL_BAD_ATTRIBUTE, "attribute 0");
        case(&[EGL_DONT_CARE], EGL_BAD_ATTRIBUTE, "a list of one bad name");
        /* OpenGL-only attributes: this EGL has OpenGL ES contexts alone */
        case(&[EGL_CONTEXT_OPENGL_PROFILE_MASK, 1, EGL_NONE], EGL_BAD_ATTRIBUTE, "a GL profile");
        case(&[EGL_CONTEXT_OPENGL_FORWARD_COMPATIBLE, EGL_TRUE, EGL_NONE], EGL_BAD_ATTRIBUTE, "forward compatible");
        case(&[EGL_CONTEXT_OPENGL_DEBUG, 7, EGL_NONE], EGL_BAD_ATTRIBUTE, "a boolean that is not one");
        /* asked for properly, but this EGL cannot give them */
        case(&[EGL_CONTEXT_OPENGL_ROBUST_ACCESS, EGL_TRUE, EGL_NONE], EGL_BAD_MATCH, "robust buffer access");
        let lose = [EGL_CONTEXT_OPENGL_RESET_NOTIFICATION_STRATEGY, EGL_LOSE_CONTEXT_ON_RESET, EGL_NONE];
        case(&lose, EGL_BAD_MATCH, "contexts are never lost");
        /* EGL_KHR_create_context's flags */
        case(&[EGL_CONTEXT_FLAGS_KHR, 0x2, EGL_NONE], EGL_BAD_ATTRIBUTE, "forward compatible is OpenGL's");
        case(&[EGL_CONTEXT_FLAGS_KHR, 0x8, EGL_NONE], EGL_BAD_ATTRIBUTE, "an undefined flag");
        case(&[EGL_CONTEXT_FLAGS_KHR, 0x4, EGL_NONE], EGL_BAD_MATCH, "robust access by flag");
        assert!(!create(&[EGL_CONTEXT_FLAGS_KHR, 0x1, EGL_CONTEXT_MAJOR_VERSION, 3, EGL_NONE]).is_null());
        assert_eq!(egl_error(), EGL_SUCCESS);

        let ok = [
            EGL_CONTEXT_MAJOR_VERSION,
            3,
            EGL_CONTEXT_MINOR_VERSION,
            0,
            EGL_CONTEXT_OPENGL_DEBUG,
            EGL_TRUE,
            EGL_CONTEXT_OPENGL_ROBUST_ACCESS,
            EGL_FALSE,
            EGL_CONTEXT_OPENGL_RESET_NOTIFICATION_STRATEGY,
            EGL_NO_RESET_NOTIFICATION,
            EGL_NONE,
        ];
        assert!(!create(&ok).is_null());
        assert_eq!(egl_error(), EGL_SUCCESS);
    }

    #[test]
    fn create_context_errors() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        assert!(egl.context(core::ptr::null_mut()).is_null());
        assert_eq!(egl_error(), EGL_BAD_CONFIG);
        let attrs = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
        // SAFETY: an EGL_NONE-terminated list.
        assert!(unsafe { eglCreateContext(0x1 as EGLDisplay, CONFIG_1X, NO_CONTEXT, attrs.as_ptr()) }.is_null());
        assert_eq!(egl_error(), EGL_BAD_DISPLAY);
    }

    #[test]
    fn context_pool_holds_max_contexts() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let c: Vec<EGLContext> = (0..MAX_CONTEXTS).map(|_| egl.context(CONFIG_1X)).collect();
        assert!(c.iter().all(|c| !c.is_null()));
        assert_ne!(c[0], c[1]);
        assert!(egl.context(CONFIG_1X).is_null());
        assert_eq!(egl_error(), EGL_BAD_ALLOC);
        // SAFETY: plain EGL call.
        assert_eq!(unsafe { eglDestroyContext(egl.dpy, c[0]) }, TRUE);
        assert!(!egl.context(CONFIG_1X).is_null());
    }

    /// hwrt_thrash's sequence: one context, one program linked once, then the
    /// context re-bound to a succession of pbuffers of different sizes, with a
    /// release in between, drawing into each. On the board every one of those
    /// draws raised GL_INVALID_OPERATION from current_drawable_program() and
    /// the surface came back holding the clear colour, in every arrangement of
    /// the MMU parameters -- so it is not the firmware fault the fence
    /// timeouts were. This pins where the program state is lost.
    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's shader names */
    fn a_program_survives_rebinding_the_context_to_another_surface() {
        const TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let c = egl.context(CONFIG_1X);
        let s0 = egl.pbuffer(CONFIG_1X, 16, 16);
        assert_eq!(egl.make_current(s0, c), TRUE);

        let prog = egl.gl.program("passthrough.vert", "uniform_color.frag");
        // SAFETY: plain GL calls on valid names; TRI outlives the draws.
        unsafe {
            crate::gl::program::glUseProgram(prog);
            crate::gl::vertex::glEnableVertexAttribArray(0);
        }
        assert_eq!(gl_error(), GL_NO_ERROR, "setup");

        /* The sizes hwrt_thrash cycles, including a non-tile-multiple one.
         * Only a few pbuffers fit in the pool, so the previous one is
         * destroyed once it is no longer current, as the harness does. */
        let mut prev = s0;
        for (w, h) in [(32, 24), (48, 33), (99, 110), (16, 16)] {
            assert_eq!(egl.release(), TRUE, "release before {w}x{h}");
            // SAFETY: plain EGL call; `prev` is live and not current.
            unsafe { eglDestroySurface(egl.dpy, prev) };
            let s = egl.pbuffer(CONFIG_1X, w, h);
            assert!(!s.is_null(), "pbuffer {w}x{h}");
            assert_eq!(egl.make_current(s, c), TRUE, "make current {w}x{h}");

            /* The context must still have the program it was given. */
            let cur = context::current().expect("a current context");
            assert_eq!(cur.cur_program, prog, "the current program was lost at {w}x{h}");
            assert_eq!(cur.target_w, w as u32, "the target width did not follow the surface");
            assert_eq!(cur.target_h, h as u32, "the target height did not follow the surface");

            // SAFETY: plain GL calls; TRI is live for the call.
            unsafe {
                crate::gl::vertex::glVertexAttribPointer(
                    0,
                    4,
                    crate::gl::consts::GL_FLOAT,
                    0,
                    0,
                    TRI.as_ptr() as *const core::ffi::c_void,
                );
                crate::gl::draw::glDrawArrays(crate::gl::consts::GL_TRIANGLES, 0, 3);
            }
            assert_eq!(gl_error(), GL_NO_ERROR, "the draw into {w}x{h} raised an error");
            prev = s;
        }
    }

    #[test]
    fn the_gl_context_is_created_by_the_first_make_current() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let c = egl.context(CONFIG_1X);
        let ci = state().lookup_context(c).unwrap();
        assert!(state().ctx[ci].gl.is_null());
        assert!(gl.mem_events().is_empty(), "creating a context allocates nothing");
        let s = egl.pbuffer(CONFIG_1X, 12, 6);
        assert_eq!(egl.make_current(s, c), TRUE);
        let glc = state().ctx[ci].gl;
        // SAFETY: by-value read of the current-context pointer.
        assert!(!glc.is_null() && glc == unsafe { context::gl_ctx });
        assert!(!core::ptr::eq(glc, gl.ctx()), "not TestGl's context");
        /* GL state starts from its defaults, sized by the surface */
        let glc = context::current().unwrap();
        assert_eq!(glc.viewport, [0, 0, 12, 6]);
        assert_eq!(glc.scissor, [0, 0, 12, 6]);
        assert_eq!(glc.default_surface.gpu, surf(s).mem.gpu);
        assert!(!glc.default_surface_owned);
        assert_eq!((glc.default_format, glc.default_samples), (GLES_SURF_RGBA8888, 1));
    }

    #[test]
    fn destroy_context_frees_the_gl_context_and_kills_the_handle() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (s, c) = egl.current(4, 4);
        texture(4, 4);
        assert_eq!(gl.live_allocs(), 3);
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglDestroyContext(egl.dpy, c), TRUE);
            assert_eq!(eglGetCurrentContext(), NO_CONTEXT, "destroying the current context releases it");
            assert!(context::gl_ctx.is_null());
            assert_eq!(gl.live_allocs(), 2, "the context's texture went with it; the pbuffer stays");
            assert_eq!(eglDestroyContext(egl.dpy, c), FALSE);
            assert_eq!(egl_error(), EGL_BAD_CONTEXT);
            assert_eq!(eglDestroyContext(egl.dpy, NO_CONTEXT), FALSE);
            assert_eq!(egl_error(), EGL_BAD_CONTEXT);
            assert_eq!(eglDestroyContext(0x1 as EGLDisplay, c), FALSE);
            assert_eq!(egl_error(), EGL_BAD_DISPLAY);
        }
        /* its GL pool slot serves the next context */
        let c2 = egl.context(CONFIG_1X);
        assert_eq!(egl.make_current(s, c2), TRUE);
    }

    #[test]
    fn destroying_a_context_never_made_current_is_fine() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let c = egl.context(CONFIG_1X);
        // SAFETY: plain EGL call.
        assert_eq!(unsafe { eglDestroyContext(egl.dpy, c) }, TRUE);
        assert!(gl.mem_events().is_empty());
        assert!(core::ptr::eq(context::current().unwrap(), gl.ctx()), "TestGl's context is still current");
    }

    #[test]
    fn query_context_attributes() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let c = egl.context(CONFIG_1X);
        assert_eq!(query_context(egl.dpy, c, EGL_CONTEXT_CLIENT_TYPE), Ok(EGL_OPENGL_ES_API));
        assert_eq!(query_context(egl.dpy, c, EGL_CONTEXT_CLIENT_VERSION), Ok(3));
        assert_eq!(query_context(egl.dpy, c, EGL_CONFIG_ID), Ok(1));
        assert_eq!(query_context(egl.dpy, c, EGL_RENDER_BUFFER), Ok(EGL_NONE), "not bound to a surface");
        let s = egl.pbuffer(CONFIG_1X, 4, 4);
        assert_eq!(egl.make_current(s, c), TRUE);
        /* the answer follows the bound surface's kind: a pbuffer renders to
         * the back buffer (EGL 1.5 table 3.5), a window in place */
        assert_eq!(query_context(egl.dpy, c, EGL_RENDER_BUFFER), Ok(EGL_BACK_BUFFER));
        assert_eq!(egl.release(), TRUE);
        assert_eq!(query_context(egl.dpy, c, EGL_RENDER_BUFFER), Ok(EGL_NONE));
    }

    #[test]
    fn query_context_config_id_is_the_creating_config() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let c = egl.context(CONFIG_565);
        assert_eq!(query_context(egl.dpy, c, EGL_CONFIG_ID), Ok(3));
    }

    #[test]
    fn query_context_errors() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let c = egl.context(CONFIG_1X);
        assert_eq!(query_context(egl.dpy, c, EGL_WIDTH), Err(EGL_BAD_ATTRIBUTE));
        assert_eq!(query_context(egl.dpy, NO_CONTEXT, EGL_CONFIG_ID), Err(EGL_BAD_CONTEXT));
        assert_eq!(query_context(0x1 as EGLDisplay, c, EGL_CONFIG_ID), Err(EGL_BAD_DISPLAY));
        // SAFETY: null is rejected.
        assert_eq!(unsafe { eglQueryContext(egl.dpy, c, EGL_CONFIG_ID, core::ptr::null_mut()) }, FALSE);
        assert_eq!(egl_error(), EGL_BAD_PARAMETER);
    }

    /* ---- eglMakeCurrent ------------------------------------------------------------------------------------- */

    #[test]
    fn make_current_binds_and_get_current_reports_it() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglGetCurrentContext(), NO_CONTEXT);
            assert_eq!(eglGetCurrentDisplay(), NO_DISPLAY);
            assert_eq!(eglGetCurrentSurface(EGL_DRAW), NO_SURFACE);
        }
        let (s, c) = egl.current(4, 4);
        assert_eq!(egl_error(), EGL_SUCCESS);
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglGetCurrentContext(), c);
            assert_eq!(eglGetCurrentDisplay(), egl.dpy);
            assert_eq!(eglGetCurrentSurface(EGL_DRAW), s);
            assert_eq!(eglGetCurrentSurface(EGL_READ), s);
        }
        assert_eq!((gl.flushes(), gl.finishes()), (0, 0), "nothing was current: nothing to submit");
    }

    /* ---- the thread contract (context::foreign_thread) ---------------------
     * One thread issues every GL and EGL call. The mock OSAL names each std
     * thread, so the thread that makes a context current owns the library
     * and a second one is refused. Handles cross to the second thread as
     * integers: raw pointers are not Send. */

    #[test]
    fn a_second_thread_is_refused_while_a_context_is_current() {
        use crate::gl::consts::{GL_BLEND, GL_INVALID_ENUM, GL_INVALID_OPERATION};
        use crate::gl::state::glEnable;
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (s, c) = egl.current(4, 4);
        // SAFETY: plain GL call on the current context.
        unsafe { glEnable(0xdead) }; /* the owner's own pending error */
        let _ = gl.logs();
        let (dpy, s_, c_) = (egl.dpy as usize, s as usize, c as usize);
        std::thread::scope(|t| {
            t.spawn(move || {
                let (dpy, s, c) = (dpy as EGLDisplay, s_ as EGLSurface, c_ as EGLContext);
                /* This thread does not hold the test lock (testutil::HOLDS_SERIAL):
                 * an EGL call that reached the EGL state would panic here, so a
                 * refusal that came too late fails the join below. */
                // SAFETY: plain GL and EGL calls; each must be refused before it
                // touches any state.
                unsafe {
                    assert_eq!(glGetError(), GL_INVALID_OPERATION, "glGetError on the second thread");
                    glClearColor(1.0, 1.0, 1.0, 1.0);
                    glEnable(GL_BLEND);
                    assert_eq!(glGetError(), GL_INVALID_OPERATION, "still refused, and nothing was taken");
                    assert_eq!(eglMakeCurrent(dpy, s, s, c), FALSE, "eglMakeCurrent on the second thread");
                    assert_eq!(eglGetError(), EGL_BAD_ACCESS);
                    assert_eq!(eglGetCurrentContext(), NO_CONTEXT, "no context is current on this thread");
                }
            })
            .join()
            .expect("the second thread's calls are refused, not run");
        });
        /* the owner is untouched: its error, its state, its EGL status */
        assert_eq!(gl_error(), GL_INVALID_ENUM, "the owner keeps its own error");
        let glc = context::current().expect("still current on the owner");
        assert_eq!(glc.clear_color, [0.0; 4], "the refused glClearColor changed nothing");
        assert!(!glc.blend, "the refused glEnable changed nothing");
        assert_eq!(egl_error(), EGL_SUCCESS);
        // SAFETY: plain EGL call.
        assert_eq!(unsafe { eglGetCurrentContext() }, c);
        let refusals: Vec<_> = gl.logs().into_iter().filter(|l| l.contains("refused")).collect();
        assert_eq!(refusals.len(), 1, "one log line for the first refusal, none after: {refusals:?}");
        assert!(refusals[0].contains("one thread per process"), "the line names the contract: {}", refusals[0]);
    }

    #[test]
    fn a_released_context_leaves_nothing_to_refuse() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        egl.current(4, 4);
        assert_eq!(egl.release(), TRUE);
        std::thread::scope(|t| {
            t.spawn(|| {
                // SAFETY: plain GL call; with no context current it does nothing.
                assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "nothing owned: not refused");
            })
            .join()
            .expect("no refusal once the owner released its context");
        });
        assert!(gl.logs().iter().all(|l| !l.contains("refused")));
    }

    #[test]
    fn nothing_is_refused_where_the_platform_cannot_name_threads() {
        use crate::gl::consts::GL_INVALID_ENUM;
        use crate::gl::state::glEnable;
        let gl = TestGl::new(8, 8);
        crate::testutil::set_thread_ids(false); /* thread_id answers 0 */
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        // SAFETY: plain GL call on the current context.
        unsafe { glEnable(0xdead) };
        let c_ = c as usize;
        std::thread::scope(|t| {
            t.spawn(move || {
                /* the owner is parked in join(), so this thread stands in for it
                 * under its test lock: the calls below are the contract breach
                 * the library cannot see on such a platform, made safe here */
                crate::testutil::HOLDS_SERIAL.with(|h| h.set(true));
                // SAFETY: plain GL and EGL calls; the owner thread is blocked.
                unsafe {
                    assert_eq!(glGetError(), GL_INVALID_ENUM, "not refused: it reads the one shared context");
                    assert_eq!(eglGetCurrentContext() as usize, c_, "not refused: the one shared EGL state");
                }
            })
            .join()
            .expect("nothing is refused without thread ids");
        });
        assert!(gl.logs().iter().all(|l| !l.contains("refused")));
    }

    #[test]
    fn get_current_surface_bad_selector_is_bad_parameter() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        egl.current(4, 4);
        // SAFETY: plain EGL call.
        assert_eq!(unsafe { eglGetCurrentSurface(EGL_WIDTH) }, NO_SURFACE);
        assert_eq!(egl_error(), EGL_BAD_PARAMETER);
    }

    #[test]
    fn releasing_the_context_completes_its_frame() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (s, _c) = egl.current(4, 4);
        // SAFETY: plain GL calls on the EGL context.
        unsafe {
            glClearColor(1.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        assert_eq!(egl.release(), TRUE);
        let draws = gl.draws();
        assert_eq!(draws.len(), 1, "the pending clear is emitted");
        assert!(draws[0].draw.do_clear);
        assert_eq!(draws[0].draw.target_gpu, surf(s).mem.gpu);
        assert_eq!((gl.flushes(), gl.finishes()), (1, 1));
        // SAFETY: plain EGL calls; by-value read of the current-context pointer.
        unsafe {
            assert_eq!(eglGetCurrentContext(), NO_CONTEXT);
            assert_eq!(eglGetCurrentSurface(EGL_DRAW), NO_SURFACE);
            assert_eq!(eglGetCurrentDisplay(), NO_DISPLAY);
            assert!(context::gl_ctx.is_null());
        }
        /* releasing again has nothing to submit */
        assert_eq!(egl.release(), TRUE);
        assert_eq!((gl.flushes(), gl.finishes()), (1, 1));
    }

    #[test]
    fn make_current_errors() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let s = egl.pbuffer(CONFIG_1X, 4, 4);
        let s2 = egl.pbuffer(CONFIG_1X, 4, 4);
        let c = egl.context(CONFIG_1X);
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglMakeCurrent(0x1 as EGLDisplay, s, s, c), FALSE);
            assert_eq!(egl_error(), EGL_BAD_DISPLAY);
            assert_eq!(eglMakeCurrent(egl.dpy, s, s, NO_CONTEXT), FALSE);
            assert_eq!(egl_error(), EGL_BAD_MATCH, "surfaces without a context");
            assert_eq!(eglMakeCurrent(egl.dpy, s, s2, c), FALSE);
            assert_eq!(egl_error(), EGL_BAD_MATCH, "one surface serves draw and read");
            assert_eq!(eglMakeCurrent(egl.dpy, NO_SURFACE, NO_SURFACE, c), FALSE);
            assert_eq!(egl_error(), EGL_BAD_MATCH, "no surfaceless contexts");
            /* the handles are judged before the one-surface rule: a read
             * surface that is no surface is EGL_BAD_SURFACE, not the
             * EGL_BAD_MATCH the pair would earn (EGL 1.5 3.7.3) */
            assert_eq!(eglMakeCurrent(egl.dpy, s, 0x10 as EGLSurface, c), FALSE);
            assert_eq!(egl_error(), EGL_BAD_SURFACE, "an invalid read surface");
            assert_eq!(eglMakeCurrent(egl.dpy, 0x10 as EGLSurface, 0x10 as EGLSurface, c), FALSE);
            assert_eq!(egl_error(), EGL_BAD_SURFACE);
            assert_eq!(eglMakeCurrent(egl.dpy, s, s, 0x10 as EGLContext), FALSE);
            assert_eq!(egl_error(), EGL_BAD_CONTEXT);
            assert_eq!(eglGetCurrentContext(), NO_CONTEXT);
        }
        assert!(core::ptr::eq(context::current().unwrap(), gl.ctx()), "a failed bind changes nothing");
    }

    #[test]
    fn make_current_on_a_terminated_display_is_not_initialized() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let s = egl.pbuffer(CONFIG_1X, 4, 4);
        let c = egl.context(CONFIG_1X);
        // SAFETY: plain EGL calls.
        unsafe {
            eglTerminate(egl.dpy);
            assert_eq!(eglMakeCurrent(egl.dpy, s, s, c), FALSE);
            assert_eq!(egl_error(), EGL_NOT_INITIALIZED);
            /* releasing is always allowed */
            assert_eq!(eglMakeCurrent(egl.dpy, NO_SURFACE, NO_SURFACE, NO_CONTEXT), TRUE);
        }
    }

    #[test]
    fn moving_a_context_to_another_surface_keeps_its_state_and_does_not_wait() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let s2 = egl.pbuffer(CONFIG_1X, 16, 8);
        // SAFETY: plain GL calls on the EGL context.
        unsafe {
            glClearColor(0.0, 0.5, 1.0, 1.0);
            texture(2, 2);
        }
        gl.clear_recordings();
        assert_eq!(egl.make_current(s2, c), TRUE);
        /* the frame so far is submitted, but not waited for (double buffering) */
        assert_eq!((gl.flushes(), gl.finishes()), (1, 0));
        let glc = context::current().unwrap();
        assert_eq!(glc.clear_color, [0.0, 0.5, 1.0, 1.0]);
        assert!(glc.objs().textures[1].used);
        assert_eq!((glc.target_w, glc.target_h), (16, 8));
        assert_eq!(glc.default_surface.gpu, surf(s2).mem.gpu);
        assert_eq!(glc.viewport, [0, 0, 4, 4], "the viewport is the application's to update");
        // SAFETY: plain EGL call.
        assert_eq!(unsafe { eglGetCurrentSurface(EGL_DRAW) }, s2);
    }

    #[test]
    fn a_second_context_cannot_become_current_while_the_gl_pool_is_full() {
        /* MAX_CONTEXTS GL contexts exist at once and TestGl holds one */
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (s, c) = egl.current(4, 4);
        /* a context keeps its GL pool slot once it has been current, so
         * these fill the pool whatever YAKOGL_MAX_CONTEXTS made it */
        let fill: Vec<EGLContext> = (2..MAX_CONTEXTS).map(|_| egl.context(CONFIG_1X)).collect();
        for f in &fill {
            assert_eq!(egl.make_current(s, *f), TRUE);
        }
        let last = fill.last().copied().unwrap_or(c);
        let c2 = egl.context(CONFIG_1X);
        assert!(!c2.is_null());
        assert_eq!(egl.make_current(s, c2), FALSE);
        assert_eq!(egl_error(), EGL_BAD_ALLOC);
        // SAFETY: plain EGL call.
        assert_eq!(unsafe { eglGetCurrentContext() }, last, "the context in use stays current");
    }

    #[test]
    fn switching_contexts_waits_for_the_frame_and_each_keeps_its_own_state() {
        let gl = TestGl::new(8, 8);
        let mut egl = Egl::new(&gl);
        egl.free_testgl_context();
        let (s, c1) = egl.current(4, 4);
        let c2 = egl.context(CONFIG_1X);
        // SAFETY: plain GL call on c1.
        unsafe { glClearColor(1.0, 0.0, 0.0, 1.0) };
        gl.clear_recordings();

        assert_eq!(egl.make_current(s, c2), TRUE);
        assert_eq!((gl.flushes(), gl.finishes()), (1, 1), "the contexts share the backend's frame sets");
        assert_eq!(context::current().unwrap().clear_color, [0.0; 4], "a new context starts from the defaults");
        // SAFETY: plain GL call on c2; plain EGL call.
        unsafe {
            glClearColor(0.0, 1.0, 0.0, 1.0);
            assert_eq!(eglGetCurrentContext(), c2);
        }

        assert_eq!(egl.make_current(s, c1), TRUE);
        assert_eq!(context::current().unwrap().clear_color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(egl.make_current(s, c2), TRUE);
        assert_eq!(context::current().unwrap().clear_color, [0.0, 1.0, 0.0, 1.0]);
        assert_eq!((gl.flushes(), gl.finishes()), (3, 3));
    }

    /// A clear is a command of the context that issued it (review R-04):
    /// eglMakeCurrent flushes it into that context's surface (EGL 1.5 3.7.3,
    /// "that context is flushed"), and the other context's clear goes into
    /// its own surface. Neither lands in the other's target or is lost.
    #[test]
    fn a_pending_clear_stays_with_its_context_across_egl_switches() {
        let gl = TestGl::new(8, 8);
        let mut egl = Egl::new(&gl);
        egl.free_testgl_context();
        let (sa, ca) = egl.current(4, 4);
        let sb = egl.pbuffer(CONFIG_1X, 4, 4);
        let cb = egl.context(CONFIG_1X);
        let kick = |gl: &TestGl| {
            let draws = gl.draws();
            assert_eq!(draws.len(), 1, "one clear-only kick");
            let d = draws[0].draw;
            assert!(d.do_clear);
            (d.clear_rgba, d.target_gpu)
        };
        // SAFETY: plain GL calls on A.
        unsafe {
            glClearColor(1.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        assert!(gl.draws().is_empty(), "A's clear is pending");
        assert_eq!(egl.make_current(sb, cb), TRUE);
        assert_eq!(kick(&gl), ([1.0, 0.0, 0.0, 1.0], surf(sa).mem.gpu), "A's clear, into A's surface");
        // SAFETY: plain GL calls on B.
        unsafe {
            glClearColor(0.0, 0.0, 1.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        assert_eq!(egl.make_current(sa, ca), TRUE);
        assert_eq!(kick(&gl), ([0.0, 0.0, 1.0, 1.0], surf(sb).mem.gpu), "B's clear, into B's surface");
        // SAFETY: plain GL call on A.
        unsafe { crate::gl::state::glFlush() };
        assert!(gl.draws().is_empty(), "nothing of B's is left pending in A");
    }

    /* ---- swap / wait ------------------------------------------------------------------------------------------ */

    #[test]
    fn swap_buffers_submits_the_frame_and_waits_for_the_previous_one() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (s, _c) = egl.current(4, 4);
        // SAFETY: plain GL / EGL calls.
        unsafe {
            glClearColor(0.0, 0.0, 1.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            assert_eq!(eglSwapBuffers(egl.dpy, s), TRUE);
        }
        let draws = gl.draws();
        assert_eq!(draws.len(), 1, "a clear no draw followed is emitted as a clear-only kick");
        let d = &draws[0].draw;
        assert!(d.do_clear && !d.flip_y, "egl.current() draws into a pbuffer, stored y-up");
        assert_eq!(d.clear_rgba, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!((d.target_gpu, d.target_w, d.target_h), (surf(s).mem.gpu, 4, 4));
        /* the mock backend counts finish_previous as a finish */
        assert_eq!((gl.flushes(), gl.finishes()), (1, 1));
        /* EGL 1.5 3.10.1: a pbuffer swap "has no effect": the backend does
         * not hear of it, so the pbuffer keeps its depth/stencil */
        assert_eq!(gl.swaps(), [], "no surface_swapped for a pbuffer");
        assert_eq!(egl_error(), EGL_SUCCESS);
    }

    #[test]
    fn a_window_swap_tells_the_backend_before_its_flush_and_a_pbuffer_swap_does_not() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let mut win = window(GLES_NATIVE_FORMAT_XRGB8888, 8, 4);
        let ws = window_surface(egl.dpy, CONFIG_1X, &mut win);
        let pb = egl.pbuffer(CONFIG_1X, 8, 4);
        let c = egl.context(CONFIG_1X);
        assert!(!ws.is_null() && !pb.is_null() && !c.is_null());
        // SAFETY: plain GL / EGL calls.
        unsafe {
            assert_eq!(egl.make_current(ws, c), TRUE);
            gl.clear_recordings();
            glClear(GL_COLOR_BUFFER_BIT);
            assert_eq!(eglSwapBuffers(egl.dpy, ws), TRUE);
            /* EGL 1.5 3.10.1: the window's depth/stencil are undefined after
             * the swap, and the backend hears of it before the flush */
            assert_eq!(gl.swaps(), [(surf(ws).mem.gpu, 0)], "surface_swapped, then the flush");
            assert_eq!(egl.make_current(pb, c), TRUE);
            glClear(GL_COLOR_BUFFER_BIT);
            assert_eq!(eglSwapBuffers(egl.dpy, pb), TRUE);
            assert_eq!(gl.swaps(), [], "a pbuffer swap has no effect");
        }
        assert_eq!(egl_error(), EGL_SUCCESS);
    }

    #[test]
    fn a_swap_waits_out_the_interval_it_was_given() {
        /* EGL 1.5 3.10.3: a swap waits `swap_interval` vertical periods, and
         * the default is 1. Without this the loop runs at whatever rate the
         * GPU allows and the display shows what a scan-out happens to catch:
         * on the AM62P 190 swaps of 250 returned in about 0.1 ms. */
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (s, _c) = egl.current(4, 4);

        gl.vblanks();
        // SAFETY: plain EGL call on a current window surface.
        unsafe { eglSwapBuffers(egl.dpy, s) };
        assert_eq!(gl.vblanks(), 1, "the default interval is one period");

        // SAFETY: plain EGL calls.
        unsafe { assert_eq!(eglSwapInterval(egl.dpy, 2), TRUE) };
        // SAFETY: as above.
        unsafe { eglSwapBuffers(egl.dpy, s) };
        assert_eq!(gl.vblanks(), 2);

        /* 0 is "do not wait", which is what a benchmark asks for */
        // SAFETY: plain EGL calls.
        unsafe { assert_eq!(eglSwapInterval(egl.dpy, 0), TRUE) };
        // SAFETY: as above.
        unsafe { eglSwapBuffers(egl.dpy, s) };
        assert_eq!(gl.vblanks(), 0);
        assert_eq!(egl_error(), EGL_SUCCESS);
    }

    #[test]
    fn a_platform_without_vertical_sync_presents_immediately() {
        /* the op answers -ENOTSUP, the swap does not spin on it, and
         * eglSwapInterval still succeeds: accept and ignore, which is what
         * every YakoGL platform did before the op existed */
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (s, _c) = egl.current(4, 4);

        gl.set_vblank_source(false);
        gl.vblanks();
        // SAFETY: plain EGL calls.
        unsafe { assert_eq!(eglSwapInterval(egl.dpy, 1), TRUE) };
        // SAFETY: as above.
        unsafe { assert_eq!(eglSwapBuffers(egl.dpy, s), TRUE) };
        assert_eq!(gl.vblanks(), 0, "nothing to wait for");
        assert_eq!(egl_error(), EGL_SUCCESS);
        gl.set_vblank_source(true);
    }

    #[test]
    fn swap_buffers_errors() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let other = egl.pbuffer(CONFIG_1X, 4, 4);
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglSwapBuffers(egl.dpy, other), FALSE);
            assert_eq!(egl_error(), EGL_BAD_SURFACE, "no context is current");
            let (s, _c) = egl.current(4, 4);
            assert_eq!(eglSwapBuffers(egl.dpy, other), FALSE);
            assert_eq!(egl_error(), EGL_BAD_SURFACE, "not the current draw surface");
            assert_eq!(eglSwapBuffers(egl.dpy, 0x10 as EGLSurface), FALSE);
            assert_eq!(egl_error(), EGL_BAD_SURFACE);
            assert_eq!(eglSwapBuffers(0x1 as EGLDisplay, s), FALSE);
            assert_eq!(egl_error(), EGL_BAD_DISPLAY);
        }
        assert_eq!(gl.flushes(), 0);
    }

    #[test]
    fn swap_interval_needs_a_current_context() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglSwapInterval(egl.dpy, 1), FALSE);
            assert_eq!(egl_error(), EGL_BAD_CONTEXT);
            egl.current(4, 4);
            for interval in [0, 1, 5, -1] {
                assert_eq!(eglSwapInterval(egl.dpy, interval), TRUE, "clamped silently: {interval}");
            }
            assert_eq!(eglSwapInterval(0x1 as EGLDisplay, 1), FALSE);
            assert_eq!(egl_error(), EGL_BAD_DISPLAY);
        }
        assert_eq!(gl.flushes(), 0);
    }

    #[test]
    fn wait_gl_drains_the_gpu_only_with_a_current_context() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        // SAFETY: plain GL / EGL calls.
        unsafe {
            assert_eq!(eglWaitGL(), TRUE);
            assert_eq!(eglWaitClient(), TRUE);
            assert_eq!((gl.flushes(), gl.finishes()), (0, 0));
            egl.current(4, 4);
            glClear(GL_COLOR_BUFFER_BIT);
            assert_eq!(eglWaitGL(), TRUE);
            assert_eq!((gl.flushes(), gl.finishes()), (1, 1));
            assert_eq!(gl.draws().len(), 1);
            assert_eq!(eglWaitClient(), TRUE);
            assert_eq!((gl.flushes(), gl.finishes()), (2, 2));
        }
    }

    #[test]
    fn wait_native_accepts_only_the_core_engine() {
        let gl = TestGl::new(8, 8);
        let _egl = Egl::new(&gl);
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglWaitNative(EGL_CORE_NATIVE_ENGINE), TRUE);
            assert_eq!(eglWaitNative(0), FALSE);
        }
        assert_eq!(egl_error(), EGL_BAD_PARAMETER);
        assert_eq!(gl.flushes(), 0);
    }

    /* ---- sync objects ------------------------------------------------------------------------------------------ */

    #[test]
    fn create_sync_flushes_and_fences_the_kick_just_submitted() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        egl.current(4, 4);
        // SAFETY: plain GL call.
        unsafe { glClear(GL_COLOR_BUFFER_BIT) };
        let sync = fence(egl.dpy);
        assert!(!sync.is_null());
        assert_eq!(gl.flushes(), 1, "EGL_SYNC_PRIOR_COMMANDS_COMPLETE: the commands so far are submitted");
        assert_eq!(gl.finishes(), 0);
        assert_eq!(gl.draws().len(), 1, "including a pending clear");
        let i = state().lookup_sync(sync).unwrap();
        /* kick_id() is the kick being recorded: the one submitted is the one before */
        assert_eq!(state().syncs[i].kick, backend::kick_id() - 1);
    }

    #[test]
    fn create_sync_errors() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        assert!(fence(egl.dpy).is_null());
        assert_eq!(egl_error(), EGL_BAD_MATCH, "no current context");
        egl.current(4, 4);
        // SAFETY: plain EGL calls.
        unsafe {
            assert!(eglCreateSync(egl.dpy, 0x30FA /* EGL_SYNC_REUSABLE_KHR */, core::ptr::null()).is_null());
            assert_eq!(egl_error(), EGL_BAD_ATTRIBUTE);
            assert!(eglCreateSync(0x1 as EGLDisplay, EGL_SYNC_FENCE as EGLenum, core::ptr::null()).is_null());
            assert_eq!(egl_error(), EGL_BAD_DISPLAY);
        }
        assert_eq!(gl.flushes(), 0);
        assert!(state().syncs.iter().all(|s| !s.used));
    }

    #[test]
    fn client_wait_sync_on_a_completed_kick_is_satisfied_without_waiting() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        egl.current(4, 4);
        let sync = fence(egl.dpy);
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglClientWaitSync(egl.dpy, sync, 0, 0), EGL_CONDITION_SATISFIED);
            assert_eq!(
                eglClientWaitSync(egl.dpy, sync, EGL_SYNC_FLUSH_COMMANDS_BIT, EGL_FOREVER),
                EGL_CONDITION_SATISFIED
            );
        }
        assert_eq!((gl.flushes(), gl.finishes()), (1, 0), "only eglCreateSync flushed");
    }

    #[test]
    fn client_wait_sync_on_a_running_kick_times_out_or_waits() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        egl.current(4, 4);
        let sync = fence(egl.dpy);
        let kick = backend::kick_id() - 1;
        gl.set_kick_pending(kick, true);
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglClientWaitSync(egl.dpy, sync, 0, 0), EGL_TIMEOUT_EXPIRED);
            assert_eq!(gl.finishes(), 0, "a zero timeout only polls");
            /* no bounded wait in the backend contract: any timeout waits for the kick */
            assert_eq!(eglClientWaitSync(egl.dpy, sync, 0, 1000), EGL_CONDITION_SATISFIED);
            assert_eq!(gl.finishes(), 1);
            assert_eq!(eglClientWaitSync(egl.dpy, sync, 0, EGL_FOREVER), EGL_CONDITION_SATISFIED);
            assert_eq!(gl.finishes(), 2);
        }
        assert_eq!(egl_error(), EGL_SUCCESS);
    }

    /// A backend that waits for one kick (`wait_kick`): eglClientWaitSync
    /// waits for the sync's kick alone and submits nothing (EGL 1.5
    /// 3.8.1.3 flushes only with EGL_SYNC_FLUSH_COMMANDS_BIT, and the fence
    /// was flushed at creation). DEMO13's direct loop recorded the next
    /// frame before this wait, and finish() submitted it into the half on
    /// screen. Revert check: the wait back on finish() counts a finish and
    /// calls no wait_kick.
    #[test]
    fn client_wait_sync_waits_for_its_kick_alone_and_submits_nothing() {
        let gl = TestGl::new(8, 8);
        gl.backend_waits_for_kicks();
        let egl = Egl::new(&gl);
        egl.current(4, 4);
        let sync = fence(egl.dpy);
        let kick = backend::kick_id() - 1;
        gl.set_kick_pending(kick, true);
        let flushes = gl.flushes();
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglClientWaitSync(egl.dpy, sync, 0, EGL_FOREVER), EGL_CONDITION_SATISFIED);
        }
        assert_eq!(gl.waited_kicks(), [kick], "the sync's own kick");
        assert_eq!((gl.flushes(), gl.finishes()), (flushes, 0), "nothing submitted, no finish");
        assert!(!gl.kick_pending(kick));
        assert_eq!(egl_error(), EGL_SUCCESS);
    }

    #[test]
    fn get_sync_attrib_reports_a_fence_and_its_status() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        egl.current(4, 4);
        let sync = fence(egl.dpy);
        let kick = backend::kick_id() - 1;
        let attr = |a| {
            let mut v: EGLAttrib = -12345;
            // SAFETY: `v` is writable.
            assert_eq!(unsafe { eglGetSyncAttrib(egl.dpy, sync, a, &mut v) }, TRUE);
            v
        };
        assert_eq!(attr(EGL_SYNC_TYPE), EGL_SYNC_FENCE as EGLAttrib);
        assert_eq!(attr(EGL_SYNC_CONDITION), EGL_SYNC_PRIOR_COMMANDS_COMPLETE as EGLAttrib);
        assert_eq!(attr(EGL_SYNC_STATUS), EGL_SIGNALED as EGLAttrib);
        gl.set_kick_pending(kick, true);
        assert_eq!(attr(EGL_SYNC_STATUS), EGL_UNSIGNALED as EGLAttrib);
        gl.set_kick_pending(kick, false);
        assert_eq!(attr(EGL_SYNC_STATUS), EGL_SIGNALED as EGLAttrib);
    }

    #[test]
    fn get_sync_attrib_errors() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        egl.current(4, 4);
        let sync = fence(egl.dpy);
        let mut v: EGLAttrib = -7;
        // SAFETY: `v` is writable; null is rejected.
        unsafe {
            assert_eq!(eglGetSyncAttrib(egl.dpy, sync, EGL_WIDTH, &mut v), FALSE);
            assert_eq!(egl_error(), EGL_BAD_ATTRIBUTE);
            assert_eq!(eglGetSyncAttrib(egl.dpy, sync, EGL_SYNC_TYPE, core::ptr::null_mut()), FALSE);
            assert_eq!(egl_error(), EGL_BAD_PARAMETER);
            assert_eq!(eglGetSyncAttrib(egl.dpy, NO_SYNC, EGL_SYNC_TYPE, &mut v), FALSE);
            assert_eq!(egl_error(), EGL_BAD_PARAMETER);
            assert_eq!(eglGetSyncAttrib(0x1 as EGLDisplay, sync, EGL_SYNC_TYPE, &mut v), FALSE);
            assert_eq!(egl_error(), EGL_BAD_DISPLAY);
        }
        assert_eq!(v, -7);
    }

    #[test]
    fn destroy_sync_kills_the_handle() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        egl.current(4, 4);
        let sync = fence(egl.dpy);
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglWaitSync(egl.dpy, sync, 0), TRUE);
            assert_eq!(eglDestroySync(egl.dpy, sync), TRUE);
            assert_eq!(eglDestroySync(egl.dpy, sync), FALSE);
            assert_eq!(egl_error(), EGL_BAD_PARAMETER);
            assert_eq!(eglClientWaitSync(egl.dpy, sync, 0, 0), EGL_FALSE);
            assert_eq!(egl_error(), EGL_BAD_PARAMETER);
            assert_eq!(eglWaitSync(egl.dpy, sync, 0), FALSE);
            assert_eq!(egl_error(), EGL_BAD_PARAMETER);
            assert_eq!(eglDestroySync(0x1 as EGLDisplay, sync), FALSE);
            assert_eq!(egl_error(), EGL_BAD_DISPLAY);
            assert_eq!(eglClientWaitSync(0x1 as EGLDisplay, sync, 0, 0), EGL_FALSE);
            assert_eq!(egl_error(), EGL_BAD_DISPLAY);
            assert_eq!(eglWaitSync(0x1 as EGLDisplay, sync, 0), FALSE);
            assert_eq!(egl_error(), EGL_BAD_DISPLAY);
        }
    }

    #[test]
    fn sync_pool_holds_eight() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        egl.current(4, 4);
        let syncs: Vec<EGLSync> = (0..EGL_MAX_SYNCS).map(|_| fence(egl.dpy)).collect();
        assert!(syncs.iter().all(|s| !s.is_null()));
        assert!(syncs.iter().enumerate().all(|(i, a)| syncs[..i].iter().all(|b| a != b)), "distinct handles");
        let flushes = gl.flushes();
        assert!(fence(egl.dpy).is_null());
        assert_eq!(egl_error(), EGL_BAD_ALLOC);
        assert_eq!(gl.flushes(), flushes, "a refused sync submits nothing");
        // SAFETY: plain EGL call.
        assert_eq!(unsafe { eglDestroySync(egl.dpy, syncs[3]) }, TRUE);
        assert_eq!(fence(egl.dpy), syncs[3], "the freed slot is reused");
    }

    /* ---- images ------------------------------------------------------------------------------------------------- */

    #[test]
    fn image_from_a_texture_takes_over_its_storage() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let tex = texture(8, 2);
        let gpu = context::current().unwrap().objs().textures[tex as usize].mem.gpu;
        gl.mem_events();
        let img = image_from_texture(egl.dpy, c, tex);
        assert!(!img.is_null(), "0x{:x}", egl_error());
        assert!(gl.mem_events().is_empty(), "no copy");
        let i = image(img);
        assert!(i.owned);
        assert_eq!((i.width, i.height, i.format, i.mem.gpu), (8, 2, GLES_SURF_RGBA8888, gpu));
        let t = &context::current().unwrap().objs().textures[tex as usize];
        assert!(t.external, "the texture keeps using storage it no longer owns");
        assert_eq!(t.mem.gpu, gpu);
    }

    #[test]
    fn image_from_a_cube_face_describes_that_faces_rows() {
        /* EGL 1.5 3.9 has the cube-map targets in the core, so a 1.5 EGL
         * answers for them; the six faces are stacked in one allocation and
         * the image is the rows of one of them. */
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let tex = cube_texture(8, 8);
        let t = &context::current().unwrap().objs().textures[tex as usize];
        let (base, face_bytes) = (t.mem.gpu, t.height as u64 * t.level_pitch(t.width) as u64 * 4);
        gl.mem_events();
        let img = image_from_face(egl.dpy, c, tex, 3);
        assert!(!img.is_null(), "0x{:x}", egl_error());
        assert!(gl.mem_events().is_empty(), "no copy");
        let i = image(img);
        assert_eq!((i.width, i.height), (8, 8));
        assert_eq!(i.mem.gpu, base + 3 * face_bytes, "face 3 of six");
        assert_eq!(i.mem.size as u64, face_bytes, "one face, not the allocation");
        assert!(i.owned, "the image owns the allocation its face is part of");
        assert_eq!(i.alloc.gpu, base);
        assert!(context::current().unwrap().objs().textures[tex as usize].external);
    }

    #[test]
    fn two_faces_of_one_cube_map_share_its_storage_record() {
        /* the six faces are one allocation: the second image has to find
         * the record the first one holds, or the allocation goes while the
         * other still points into it */
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let tex = cube_texture(8, 8);
        let live = gl.live_allocs();
        let a = image_from_face(egl.dpy, c, tex, 0);
        let b = image_from_face(egl.dpy, c, tex, 5);
        assert!(!a.is_null() && !b.is_null(), "0x{:x}", egl_error());
        let (ai, bi) = (state().lookup_image(a).unwrap(), state().lookup_image(b).unwrap());
        assert_eq!(state().img[bi].owner, Some(ai as u8), "one record, held by the first");
        assert!(!state().img[bi].owned);
        // SAFETY: plain EGL calls on live images.
        unsafe {
            assert_eq!(eglDestroyImage(egl.dpy, a), TRUE);
            assert_eq!(eglDestroyImage(egl.dpy, b), TRUE);
        }
        assert_eq!(gl.live_allocs(), live, "the texture is still a user");
        // SAFETY: room for one name.
        unsafe { glDeleteTextures(1, &tex) };
        assert_eq!(gl.live_allocs(), live - 1, "freed once, with the last user");
    }

    #[test]
    fn image_from_a_colour_renderbuffer_takes_over_its_storage() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let rb = renderbuffer(GL_RGBA4, 8, 2);
        let gpu = context::current().unwrap().objs().renderbuffers[rb as usize].mem.gpu;
        gl.mem_events();
        // SAFETY: the client buffer of EGL_GL_RENDERBUFFER is the name.
        let img = unsafe {
            eglCreateImage(
                egl.dpy,
                c,
                EGL_GL_RENDERBUFFER as EGLenum,
                rb as usize as EGLClientBuffer,
                core::ptr::null(),
            )
        };
        assert!(!img.is_null(), "0x{:x}", egl_error());
        assert!(gl.mem_events().is_empty(), "no copy");
        let i = image(img);
        /* the storage is RGBA8888 whatever format was asked for */
        assert_eq!((i.width, i.height, i.format, i.mem.gpu), (8, 2, GLES_SURF_RGBA8888, gpu));
        assert!(i.owned);
        assert!(context::current().unwrap().objs().renderbuffers[rb as usize].external);
    }

    #[test]
    fn a_depth_renderbuffer_has_no_storage_for_an_image() {
        /* the depth buffer lives with the tile, not in memory, so there is
         * nothing for an image to share */
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let rb = renderbuffer(GL_DEPTH_COMPONENT16, 8, 2);
        // SAFETY: the client buffer of EGL_GL_RENDERBUFFER is the name.
        let img = unsafe {
            eglCreateImage(
                egl.dpy,
                c,
                EGL_GL_RENDERBUFFER as EGLenum,
                rb as usize as EGLClientBuffer,
                core::ptr::null(),
            )
        };
        assert!(img.is_null());
        assert_eq!(egl_error(), EGL_BAD_PARAMETER);
    }

    #[test]
    fn every_egl_entry_point_takes_the_shared_lock_once_and_gives_it_back() {
        /* EGL 1.5 2.3: one display and its objects, several threads. The
         * lock the OSAL supplies is not recursive, so an entry point that
         * reached another entry point instead of the function under it
         * would deadlock the second thread that ran it -- a hang on the
         * board, where it is expensive to find. The mock counts instead:
         * a depth of 2 is that bug, and a depth left above 0 is a lock an
         * entry point never gave back.
         *
         * The calls below are the ones that used to nest (each entered a
         * sibling entry point), plus a spread of ordinary ones. */
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (s, c) = egl.current(4, 4);
        let tex = texture(8, 2);
        crate::testutil::lock_counts_reset();
        // SAFETY: plain EGL calls with live handles; the lists are
        // EGL_NONE-terminated or null.
        unsafe {
            /* the five that used to call a sibling entry point */
            eglGetPlatformDisplay(0, core::ptr::null_mut(), core::ptr::null());
            eglCreatePlatformPixmapSurface(egl.dpy, CONFIG_1X, core::ptr::null_mut(), core::ptr::null());
            eglReleaseTexImage(egl.dpy, s, EGL_BACK_BUFFER);
            eglWaitClient();
            let img = eglCreateImageKHR(
                egl.dpy,
                c,
                EGL_GL_TEXTURE_2D as EGLenum,
                tex as usize as EGLClientBuffer,
                core::ptr::null(),
            );
            assert!(!img.is_null());
            assert_eq!(eglDestroyImageKHR(egl.dpy, img), TRUE);
            /* and a spread of the rest */
            eglGetDisplay(core::ptr::null_mut());
            eglInitialize(egl.dpy, core::ptr::null_mut(), core::ptr::null_mut());
            eglQueryString(egl.dpy, EGL_VENDOR);
            eglGetError();
            eglBindAPI(EGL_OPENGL_ES_API as EGLenum);
            eglQueryAPI();
            eglChooseConfig(egl.dpy, [EGL_NONE].as_ptr(), core::ptr::null_mut(), 0, &mut 0);
            eglGetConfigs(egl.dpy, core::ptr::null_mut(), 0, &mut 0);
            eglCreatePbufferSurface(egl.dpy, CONFIG_1X, [EGL_WIDTH, 4, EGL_HEIGHT, 4, EGL_NONE].as_ptr());
            eglQuerySurface(egl.dpy, s, EGL_WIDTH, &mut 0);
            eglSurfaceAttrib(egl.dpy, s, EGL_MIPMAP_LEVEL, 0);
            eglQueryContext(egl.dpy, c, EGL_CONFIG_ID, &mut 0);
            eglGetCurrentContext();
            eglGetCurrentSurface(EGL_DRAW);
            eglGetCurrentDisplay();
            eglSwapInterval(egl.dpy, 1);
            eglWaitGL();
            eglWaitNative(EGL_CORE_NATIVE_ENGINE);
            eglGetProcAddress(c"eglSwapBuffers".as_ptr());
            eglMakeCurrent(egl.dpy, s, s, c);
            /* the GL side of the image table takes it too */
            glEGLImageTargetTexture2DOES(GL_TEXTURE_2D, core::ptr::null_mut());
        }
        /* and freeing image-backed storage from GL, which no EGL call holds
         * the lock across */
        // SAFETY: room for one name.
        unsafe { glDeleteTextures(1, &tex) };

        let (taken, depth, max) = crate::testutil::lock_counts();
        assert!(taken >= 25, "only {taken} of the calls above took the lock");
        assert_eq!(depth, 0, "an entry point kept the lock");
        assert_eq!(max, 1, "an entry point took the lock while holding it: a real mutex would hang");
    }

    #[test]
    fn the_khr_entry_points_create_and_destroy_the_same_images() {
        /* EGL_KHR_image_base is advertised, so its two entry points have to
         * exist and work; dEQP asks eglGetProcAddress for the functions of
         * every extension the EGL_EXTENSIONS string names. */
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let tex = texture(8, 2);
        let gpu = context::current().unwrap().objs().textures[tex as usize].mem.gpu;
        // SAFETY: the client buffer of EGL_GL_TEXTURE_2D is the texture name;
        // the KHR attribute list is EGLint and may be null.
        let img = unsafe {
            eglCreateImageKHR(
                egl.dpy,
                c,
                EGL_GL_TEXTURE_2D as EGLenum,
                tex as usize as EGLClientBuffer,
                core::ptr::null(),
            )
        };
        assert!(!img.is_null(), "0x{:x}", egl_error());
        let i = image(img);
        assert_eq!((i.width, i.height, i.mem.gpu), (8, 2, gpu));
        // SAFETY: a live image of this display.
        assert_eq!(unsafe { eglDestroyImageKHR(egl.dpy, img) }, TRUE);
        assert_eq!(egl_error(), EGL_SUCCESS);
    }

    #[test]
    fn image_storage_outlives_the_texture_and_is_freed_once_with_the_image() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let tex = texture(4, 4);
        let gpu = context::current().unwrap().objs().textures[tex as usize].mem.gpu;
        let img = image_from_texture(egl.dpy, c, tex);
        let live = gl.live_allocs();
        gl.mem_events();
        // SAFETY: one readable name; plain EGL calls.
        unsafe {
            glDeleteTextures(1, &tex);
            assert!(gl.mem_events().is_empty(), "the image owns the storage now");
            assert_eq!(eglDestroyImage(egl.dpy, img), TRUE);
            assert_eq!(gl.mem_events(), [MemEvent::Free { gpu }]);
            assert_eq!(eglDestroyImage(egl.dpy, img), FALSE);
            assert_eq!(egl_error(), EGL_BAD_PARAMETER);
        }
        assert_eq!(gl.live_allocs(), live - 1);
    }

    #[test]
    fn image_target_texture_shares_the_storage_without_a_double_free() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let src = texture(8, 4);
        let gpu = context::current().unwrap().objs().textures[src as usize].mem.gpu;
        let img = image_from_texture(egl.dpy, c, src);
        let dst = texture(2, 2); /* owns storage of its own, which the image replaces */
        let own = context::current().unwrap().objs().textures[dst as usize].mem.gpu;
        gl.mem_events();
        // SAFETY: `img` is a live image; `dst` is bound.
        unsafe { glEGLImageTargetTexture2DOES(GL_TEXTURE_2D, img) };
        assert_eq!(gl_error(), GL_NO_ERROR);
        assert_eq!(gl.mem_events(), [MemEvent::Free { gpu: own }]);
        {
            let t = &context::current().unwrap().objs().textures[dst as usize];
            assert!(t.external && !t.bgra);
            assert_eq!((t.mem.gpu, t.width, t.height, t.format), (gpu, 8, 4, GL_RGBA));
        }
        // SAFETY: two readable names; plain EGL call.
        unsafe {
            glDeleteTextures(2, [src, dst].as_ptr());
            assert!(gl.mem_events().is_empty(), "neither texture owns the image's storage");
            assert_eq!(eglDestroyImage(egl.dpy, img), TRUE);
        }
        assert_eq!(gl.mem_events(), [MemEvent::Free { gpu }]);
    }

    /// A texture on an image's storage that takes the pixel engine's channel
    /// order (a render into it, texture::take_pe_order) gives it to every
    /// texture of the context on that storage and to the image, so a texture
    /// attached to the image later reads the same order (frontend4's review
    /// of ee1d5894). Only on a backend whose RGBA8888 target is B,G,R,A.
    #[test]
    fn an_images_textures_share_the_pixel_engines_channel_order() {
        let gl = TestGl::new(8, 8);
        gl.backend_like_gc7000();
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let src = texture(8, 4);
        let img = image_from_texture(egl.dpy, c, src);
        let dst = texture(2, 2);
        // SAFETY: `img` is a live image; `dst` is bound.
        unsafe { glEGLImageTargetTexture2DOES(GL_TEXTURE_2D, img) };
        assert_eq!(gl_error(), GL_NO_ERROR);
        let slot = |n: GLuint| n as usize;
        {
            let ctx = context::current().unwrap();
            assert!(!ctx.objs().textures[slot(dst)].pe_order, "the source was only uploaded");
            super::texture::take_pe_order(ctx, slot(dst));
            assert!(ctx.objs().textures[slot(dst)].pe_order);
            assert!(ctx.objs().textures[slot(src)].pe_order, "the source shares the storage");
        }
        let late = texture(2, 2);
        // SAFETY: as above.
        unsafe { glEGLImageTargetTexture2DOES(GL_TEXTURE_2D, img) };
        assert!(context::current().unwrap().objs().textures[slot(late)].pe_order, "attached afterwards");
        // SAFETY: readable names; plain EGL call.
        unsafe {
            glDeleteTextures(3, [src, dst, late].as_ptr());
            assert_eq!(eglDestroyImage(egl.dpy, img), TRUE);
        }
    }

    #[test]
    fn a_second_image_of_the_same_texture_does_not_own_the_storage() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let tex = texture(4, 4);
        let gpu = context::current().unwrap().objs().textures[tex as usize].mem.gpu;
        let a = image_from_texture(egl.dpy, c, tex);
        let b = image_from_texture(egl.dpy, c, tex);
        assert!(!a.is_null() && !b.is_null() && a != b);
        assert!(image(a).owned && !image(b).owned);
        /* the second image refers to the first one's storage record: that
         * record has two users (the source texture and image b) */
        assert_eq!((image(a).owner, image(a).refs), (None, 2));
        assert_eq!(image(b).owner.map(|o| o as usize), state().lookup_image(a));
        gl.mem_events();
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglDestroyImage(egl.dpy, b), TRUE);
            assert!(gl.mem_events().is_empty());
            assert_eq!(eglDestroyImage(egl.dpy, a), TRUE);
            /* the source texture still samples the storage */
            assert!(gl.mem_events().is_empty(), "a texture still uses it");
            glDeleteTextures(1, &tex);
        }
        assert_eq!(gl.mem_events(), [MemEvent::Free { gpu }]);
    }

    #[test]
    fn respecifying_an_image_backed_texture_gives_it_its_own_storage_again() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let tex = texture(4, 4);
        let gpu = context::current().unwrap().objs().textures[tex as usize].mem.gpu;
        let img = image_from_texture(egl.dpy, c, tex);
        gl.mem_events();
        // SAFETY: null pixels only allocate; the texture is bound.
        unsafe {
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 4, 4, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        }
        let t = &context::current().unwrap().objs().textures[tex as usize];
        assert!(!t.external);
        assert_ne!(t.mem.gpu, gpu);
        assert!(!gl.mem_events().contains(&MemEvent::Free { gpu }), "the image's storage is left alone");
        assert_eq!(image(img).mem.gpu, gpu);
    }

    #[test]
    fn image_can_be_created_for_a_context_that_is_not_current() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let tex = texture(4, 4);
        assert_eq!(egl.release(), TRUE);
        // SAFETY: by-value reads of the current-context pointer.
        unsafe {
            assert!(context::gl_ctx.is_null());
            let img = image_from_texture(egl.dpy, c, tex);
            assert!(!img.is_null(), "0x{:x}", egl_error());
            assert!(context::gl_ctx.is_null(), "the current context is put back");
        }
    }

    #[test]
    fn create_image_errors() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let unbound = egl.context(CONFIG_1X);
        let (_s, c) = egl.current(4, 4);
        let tex = texture(4, 4);
        let mut empty: GLuint = 0;
        // SAFETY: room for one name; plain EGL calls.
        unsafe {
            glGenTextures(1, &mut empty);
            glBindTexture(GL_TEXTURE_2D, empty);

            assert!(image_from_texture(egl.dpy, unbound, tex).is_null());
            assert_eq!(egl_error(), EGL_BAD_CONTEXT, "a context that never was current has no GL objects");
            assert!(image_from_texture(egl.dpy, NO_CONTEXT, tex).is_null());
            assert_eq!(egl_error(), EGL_BAD_CONTEXT);
            assert!(image_from_texture(egl.dpy, c, empty).is_null());
            assert_eq!(egl_error(), EGL_BAD_PARAMETER, "a texture without storage");
            assert!(image_from_texture(egl.dpy, c, 200).is_null());
            assert_eq!(egl_error(), EGL_BAD_PARAMETER, "not a texture");
            assert!(image_from_texture(egl.dpy, c, 0).is_null());
            assert_eq!(egl_error(), EGL_BAD_PARAMETER);
            let buf = tex as usize as EGLClientBuffer;
            assert!(eglCreateImage(egl.dpy, c, EGL_GL_RENDERBUFFER as EGLenum, buf, core::ptr::null()).is_null());
            assert_eq!(egl_error(), EGL_BAD_PARAMETER, "an unsupported target");
            assert!(
                eglCreateImage(0x1 as EGLDisplay, c, EGL_GL_TEXTURE_2D as EGLenum, buf, core::ptr::null()).is_null()
            );
            assert_eq!(egl_error(), EGL_BAD_DISPLAY);
        }
        assert!(state().img.iter().all(|i| !i.used), "a failed create takes no slot");
        assert!(!context::current().unwrap().objs().textures[tex as usize].external);
    }

    #[test]
    fn native_pixmap_image_errors_on_a_platform_without_map_phys() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let mut nb = window(GLES_NATIVE_FORMAT_RGBA8888, 4, 4);
        let buf = &mut nb as *mut GlesNativeWindow as EGLClientBuffer;
        // SAFETY: `buf` is a valid native buffer description.
        unsafe {
            assert!(eglCreateImage(egl.dpy, c, EGL_NATIVE_PIXMAP_KHR as EGLenum, buf, core::ptr::null()).is_null());
            assert_eq!(egl_error(), EGL_BAD_CONTEXT, "a native buffer belongs to no context");
            assert!(
                eglCreateImage(egl.dpy, NO_CONTEXT, EGL_NATIVE_PIXMAP_KHR as EGLenum, buf, core::ptr::null()).is_null()
            );
            assert_eq!(egl_error(), EGL_BAD_PARAMETER, "the mock platform cannot map physical pages");
        }
    }

    #[test]
    fn native_pixmap_image_maps_the_buffer_and_targets_a_texture() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let _ops = OpsSwap::new(|o| o.map_phys = Some(t_map_phys));
        egl.current(4, 4);
        let mut pixels = std::vec![0u8; 6 * 3 * 4];
        let mut nb = window(GLES_NATIVE_FORMAT_XRGB8888, 6, 3);
        nb.phys = pixels.as_mut_ptr() as u64;
        let buf = &mut nb as *mut GlesNativeWindow as EGLClientBuffer;
        // SAFETY: `buf` is a valid native buffer description.
        let img =
            unsafe { eglCreateImage(egl.dpy, NO_CONTEXT, EGL_NATIVE_PIXMAP_KHR as EGLenum, buf, core::ptr::null()) };
        assert!(!img.is_null(), "0x{:x}", egl_error());
        let i = image(img);
        assert!(!i.owned, "the application's buffer");
        assert_eq!((i.width, i.height, i.format), (6, 3, GLES_SURF_XRGB8888));
        assert_eq!((i.mem.phys, i.mem.size), (nb.phys, 3 * 6 * 4));

        let tex = texture(2, 2);
        // SAFETY: `img` is a live image; `tex` is bound.
        unsafe { glEGLImageTargetTexture2DOES(GL_TEXTURE_2D, img) };
        assert_eq!(gl_error(), GL_NO_ERROR);
        {
            let t = &context::current().unwrap().objs().textures[tex as usize];
            assert!(t.external && t.bgra, "XRGB8888 texels are stored B,G,R,X");
            assert_eq!((t.mem.gpu, t.width, t.height), (i.mem.gpu, 6, 3));
        }
        /* only RGBA8888 images can be render targets */
        let mut rb: GLuint = 0;
        // SAFETY: room for one name; `img` is a live image.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glEGLImageTargetRenderbufferStorageOES(GL_RENDERBUFFER, img);
            assert_eq!(gl_error(), GL_INVALID_VALUE);
            glDeleteTextures(1, &tex);
        }
        gl.mem_events();
        // SAFETY: plain EGL call.
        assert_eq!(unsafe { eglDestroyImage(egl.dpy, img) }, TRUE);
        assert!(gl.mem_events().is_empty(), "nothing of the application's is freed");
    }

    #[test]
    fn native_pixmap_image_invalid_buffers_and_mapping_failure() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let _ops = OpsSwap::new(|o| o.map_phys = Some(t_map_phys));
        let mut pixels = std::vec![0u8; 4 * 4 * 4];
        let mut ok = window(GLES_NATIVE_FORMAT_RGBA8888, 4, 4);
        ok.phys = pixels.as_mut_ptr() as u64;
        let create = |nb: *mut GlesNativeWindow| {
            // SAFETY: null or a valid native buffer description.
            unsafe {
                eglCreateImage(
                    egl.dpy,
                    NO_CONTEXT,
                    EGL_NATIVE_PIXMAP_KHR as EGLenum,
                    nb as EGLClientBuffer,
                    core::ptr::null(),
                )
            }
        };
        assert!(create(core::ptr::null_mut()).is_null());
        assert_eq!(egl_error(), EGL_BAD_PARAMETER);
        let mut bad = GlesNativeWindow { stride_bytes: 20, ..ok };
        assert!(create(&mut bad).is_null());
        assert_eq!(egl_error(), EGL_BAD_PARAMETER);
        /* the scan-order flag is a window's; a pixmap is never scanned out */
        let mut flagged = GlesNativeWindow { format: ok.format | GLES_NATIVE_WINDOW_BOTTOM_UP, ..ok };
        assert!(create(&mut flagged).is_null());
        assert_eq!(egl_error(), EGL_BAD_PARAMETER);
        MAP_PHYS_FAILS.store(true, Ordering::Relaxed);
        assert!(create(&mut ok).is_null());
        assert_eq!(egl_error(), EGL_BAD_ALLOC);
        MAP_PHYS_FAILS.store(false, Ordering::Relaxed);
        assert!(state().img.iter().all(|i| !i.used));
        assert!(!create(&mut ok).is_null());
    }

    #[test]
    fn surfaces_egl_creates_are_always_large_enough_to_bind() {
        /* The invariant that keeps `gl_context_bind_surface`'s refusal
         * unreachable from the public API, asserted rather than assumed: a
         * window surface's stride is checked against `tex_pitch(width)` at
         * creation, and a pbuffer is allocated by `surface_bytes` itself. The
         * pair-padded pitch is the trap -- an odd width needs one pixel more
         * per row than `width * bpp` -- so the odd cases are here on purpose.
         * If a new surface path ever allocates differently, this fails before
         * anyone renders into the wrong buffer. */
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let mut checked = 0;
        for (cfg, format, bpp, w, h) in [
            (CONFIG_1X, GLES_SURF_RGBA8888, 4u32, 5i32, 3i32),
            (CONFIG_1X, GLES_SURF_RGBA8888, 4, 4, 4),
            (CONFIG_1X, GLES_SURF_RGBA8888, 4, 1, 1),
        ] {
            let s = egl.pbuffer(cfg, w, h);
            assert!(!s.is_null());
            let i = state().lookup_surface(s).unwrap();
            let need = context::surface_bytes(w as u32, h as u32, bpp);
            let got = state().surf[i].mem.size;
            assert!(got >= need, "pbuffer {w}x{h} format {format}: {got} bytes for {need}");
            checked += 1;
        }
        /* A window surface brings its own buffer, so the relation is checked
         * at creation instead: `native_buffer_format` demands the pair-padded
         * stride, and a packed odd-width window is EGL_BAD_NATIVE_WINDOW
         * (`a_window_surface_needs_rows_at_the_pair_pitch` pins that, byte for
         * byte). Here the surviving window only has to satisfy the same
         * relation the bind will apply to it. */
        let _ops = OpsSwap::new(|o| o.map_phys = Some(t_map_phys));
        let mut padded = std::vec![0u8; 6 * 3 * 4];
        let mut win = window(GLES_NATIVE_FORMAT_XRGB8888, 5, 3);
        win.phys = padded.as_mut_ptr() as u64;
        win.stride_bytes = 6 * 4; /* the pad pixel every odd row needs */
        let s = window_surface(egl.dpy, CONFIG_1X, &mut win);
        assert!(!s.is_null(), "0x{:x}", egl_error());
        let need = context::surface_bytes(5, 3, crate::backend::gles_surf_bytes_per_pixel(surf(s).format));
        assert!(surf(s).mem.size >= need, "window 5x3: {} bytes for {need}", surf(s).mem.size);
        checked += 1;

        assert_eq!(checked, 4, "every case ran");
        let _ = gl.error();
    }

    #[test]
    fn a_make_current_whose_bind_refuses_fails_and_leaves_the_old_surface_current() {
        /* The failure path the return value bought. It cannot be reached
         * through the public API today (the test above says why), so the
         * surface's recorded size is grown behind EGL's back to produce
         * exactly the state a window-system that hands over its own buffer
         * would. Before the bind reported, this returned EGL_TRUE and the
         * application drew into `first` while reading back `second`. */
        let gl = TestGl::new(8, 8);
        let mut egl = Egl::new(&gl);
        egl.free_testgl_context();
        let (first, ctx) = egl.current(4, 4);
        let second = egl.pbuffer(CONFIG_1X, 4, 4);
        assert!(!second.is_null());
        let si = state().lookup_surface(second).unwrap();
        state().surf[si].width = 64; /* the allocation is still 4x4 */
        state().surf[si].height = 64;

        assert_eq!(egl.make_current(second, ctx), FALSE, "a bind that cannot happen is not a success");
        assert_eq!(egl_error(), EGL_BAD_SURFACE);
        // SAFETY: plain EGL call.
        assert_eq!(unsafe { eglGetCurrentSurface(EGL_DRAW) }, first, "the old surface is still current");
        // SAFETY: plain EGL call.
        assert_eq!(unsafe { eglGetCurrentContext() }, ctx);
        let ci = state().lookup_context(ctx).unwrap();
        // SAFETY: the slot holds a live GL context (it is the current one).
        let c = unsafe { &*state().ctx[ci].gl };
        assert_eq!((c.target_w, c.target_h), (4, 4), "and the context still renders 4x4");

        /* the surface is still usable once its geometry is honest again */
        state().surf[si].width = 4;
        state().surf[si].height = 4;
        assert_eq!(egl.make_current(second, ctx), TRUE);
    }

    #[test]
    fn image_pool_holds_eight() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let tex = texture(4, 4);
        let imgs: Vec<EGLImage> = (0..EGL_MAX_IMAGES).map(|_| image_from_texture(egl.dpy, c, tex)).collect();
        assert!(imgs.iter().all(|i| !i.is_null()));
        assert!(image_from_texture(egl.dpy, c, tex).is_null());
        assert_eq!(egl_error(), EGL_BAD_ALLOC);
        // SAFETY: plain EGL call.
        assert_eq!(unsafe { eglDestroyImage(egl.dpy, imgs[5]) }, TRUE);
        assert_eq!(image_from_texture(egl.dpy, c, tex), imgs[5], "the freed slot is reused");
    }

    #[test]
    fn destroy_image_errors() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglDestroyImage(egl.dpy, NO_IMAGE), FALSE);
            assert_eq!(egl_error(), EGL_BAD_PARAMETER);
            assert_eq!(eglDestroyImage(0x1 as EGLDisplay, NO_IMAGE), FALSE);
            assert_eq!(egl_error(), EGL_BAD_DISPLAY);
        }
    }

    #[test]
    fn image_target_texture_errors() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let tex = texture(4, 4);
        let img = image_from_texture(egl.dpy, c, tex);
        // SAFETY: plain GL calls; the image handles are only looked up.
        unsafe {
            glEGLImageTargetTexture2DOES(GL_RENDERBUFFER, img);
            assert_eq!(gl_error(), GL_INVALID_ENUM);
            glEGLImageTargetTexture2DOES(GL_TEXTURE_2D, core::ptr::null_mut());
            assert_eq!(gl_error(), GL_INVALID_VALUE);
            glEGLImageTargetTexture2DOES(GL_TEXTURE_2D, 0x10 as GLeglImageOES);
            assert_eq!(gl_error(), GL_INVALID_VALUE);
            /* ES 3.0 3.8: with nothing bound the call addresses the 2D
             * target's default texture object, which takes the image */
            glBindTexture(GL_TEXTURE_2D, 0);
            glEGLImageTargetTexture2DOES(GL_TEXTURE_2D, img);
            assert_eq!(gl_error(), GL_NO_ERROR, "the default texture object");
        }
    }

    #[test]
    fn image_target_renderbuffer_storage_makes_the_image_a_render_target() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let tex = texture(8, 4);
        let gpu = context::current().unwrap().objs().textures[tex as usize].mem.gpu;
        let img = image_from_texture(egl.dpy, c, tex);
        let (mut rb, mut fb): (GLuint, GLuint) = (0, 0);
        // SAFETY: room for one name each; `img` is a live image.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA4, 2, 2);
            let own = context::current().unwrap().objs().renderbuffers[rb as usize].mem.gpu;
            gl.mem_events();
            glEGLImageTargetRenderbufferStorageOES(GL_RENDERBUFFER, img);
            assert_eq!(gl_error(), GL_NO_ERROR);
            assert_eq!(gl.mem_events(), [MemEvent::Free { gpu: own }]);
            {
                let r = &context::current().unwrap().objs().renderbuffers[rb as usize];
                assert!(r.external);
                assert_eq!((r.mem.gpu, r.width, r.height, r.internalformat), (gpu, 8, 4, GL_RGBA));
            }
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, crate::gl::consts::GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            glDeleteRenderbuffers(1, &rb);
            assert!(gl.mem_events().is_empty(), "the renderbuffer never owned the image's storage");
        }
    }

    #[test]
    fn image_target_renderbuffer_storage_errors() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let tex = texture(4, 4);
        let img = image_from_texture(egl.dpy, c, tex);
        // SAFETY: plain GL calls; the image handles are only looked up.
        unsafe {
            glEGLImageTargetRenderbufferStorageOES(GL_RENDERBUFFER, img);
            assert_eq!(gl_error(), GL_INVALID_OPERATION, "no renderbuffer bound");
            glEGLImageTargetRenderbufferStorageOES(GL_TEXTURE_2D, img);
            assert_eq!(gl_error(), GL_INVALID_ENUM);
            glEGLImageTargetRenderbufferStorageOES(GL_RENDERBUFFER, core::ptr::null_mut());
            assert_eq!(gl_error(), GL_INVALID_VALUE);
        }
    }

    #[test]
    fn image_target_calls_without_a_current_context_do_nothing() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let tex = texture(4, 4);
        let img = image_from_texture(egl.dpy, c, tex);
        assert_eq!(egl.release(), TRUE);
        gl.mem_events();
        // SAFETY: no context is current: both calls return at once.
        unsafe {
            glEGLImageTargetTexture2DOES(GL_TEXTURE_2D, img);
            glEGLImageTargetRenderbufferStorageOES(GL_RENDERBUFFER, img);
        }
        assert!(gl.mem_events().is_empty());
    }

    #[test]
    fn destroying_the_context_leaves_the_images_storage_alone() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let tex = texture(4, 4);
        let gpu = context::current().unwrap().objs().textures[tex as usize].mem.gpu;
        let img = image_from_texture(egl.dpy, c, tex);
        gl.mem_events();
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglDestroyContext(egl.dpy, c), TRUE);
            assert_eq!(eglDestroyImage(egl.dpy, img), TRUE);
        }
        /* the mock reports the second free of `gpu` here ("double free") */
        assert_eq!(gl.mem_events(), [MemEvent::Free { gpu }], "freed once, by the image");
    }

    #[test]
    fn destroying_an_image_leaves_its_sibling_textures_valid() {
        use crate::gl::consts::{GL_FLOAT, GL_TRIANGLES};
        use crate::gl::draw::glDrawArrays;
        use crate::gl::program::glUseProgram;
        use crate::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};

        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let src = texture(4, 4);
        let img = image_from_texture(egl.dpy, c, src);
        let prog = gl.program("uv.vert", "texture.frag");
        let tri: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
        let _dst = texture(2, 2);
        gl.allow_uninit_reads(true); /* the texels are undefined; their lifetime is what matters */
        // SAFETY: live image and names; `tri` outlives the draws.
        unsafe {
            glEGLImageTargetTexture2DOES(GL_TEXTURE_2D, img);
            glDeleteTextures(1, &src);
            assert_eq!(eglDestroyImage(egl.dpy, img), TRUE);
            glUseProgram(prog);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, tri.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            eglWaitGL(); /* whatever was in flight when the image went has completed */
            glDrawArrays(GL_TRIANGLES, 0, 3); /* samples `dst` */
        }
        /* the mock reports "the draw reads a texture ...: freed memory" */
        assert_eq!(gl_error(), GL_NO_ERROR);
        assert_eq!(gl.draws().len(), 1);
    }

    /* ---- eglGetProcAddress ----------------------------------------------------------------------------------------- */

    fn proc_addr(name: &CStr) -> Option<usize> {
        // SAFETY: a NUL-terminated name.
        unsafe { eglGetProcAddress(name.as_ptr()) }.map(|f| f as usize)
    }

    #[test]
    fn get_proc_address_resolves_gl_and_egl_entry_points() {
        /* eglGetProcAddress writes the shared EGL error status: hold the lock. */
        let _serial = TestGl::new(8, 8);
        for name in [
            c"glClear",
            c"glDrawElements",
            c"glTexImage2D",
            c"glReadPixels",
            c"glGetStringi",
            c"glBlitFramebuffer",
            c"glMapBufferRange",
            c"glDrawArraysInstanced",
            c"glEGLImageTargetTexture2DOES",
            c"glEGLImageTargetRenderbufferStorageOES",
            c"eglSwapBuffers",
            c"eglCreateImage",
            /* EGL_KHR_image_base, which EGL_EXTENSIONS names */
            c"eglCreateImageKHR",
            c"eglDestroyImageKHR",
            c"eglGetProcAddress",
        ] {
            assert!(proc_addr(name).is_some(), "{name:?}");
        }
    }

    #[test]
    fn get_proc_address_answers_for_every_es3_entry_point() {
        /* eglGetProcAddress writes the shared EGL error status: hold the lock. */
        let _serial = TestGl::new(8, 8);
        /* EGL 1.5 3.10: eglGetProcAddress resolves every EGL and client-API
         * function, not only the ones an extension added. A conformance run
         * resolves the whole API through it and calls what it gets back, so a
         * missing name is a jump to address zero rather than a failure it can
         * report. */
        for name in [
            c"glBeginQuery",
            c"glBeginTransformFeedback",
            c"glBindBufferBase",
            c"glBindBufferRange",
            c"glBindSampler",
            c"glBindTransformFeedback",
            c"glBindVertexArray",
            c"glClearBufferfi",
            c"glClearBufferfv",
            c"glClearBufferiv",
            c"glClearBufferuiv",
            c"glClientWaitSync",
            c"glCompressedTexSubImage3D",
            c"glCopyBufferSubData",
            c"glCopyTexSubImage3D",
            c"glDeleteQueries",
            c"glDeleteSamplers",
            c"glDeleteSync",
            c"glDeleteTransformFeedbacks",
            c"glDeleteVertexArrays",
            c"glEndQuery",
            c"glEndTransformFeedback",
            c"glFenceSync",
            c"glGenQueries",
            c"glGenSamplers",
            c"glGenTransformFeedbacks",
            c"glGenVertexArrays",
            c"glGetActiveUniformBlockName",
            c"glGetActiveUniformBlockiv",
            c"glGetActiveUniformsiv",
            c"glGetBufferParameteri64v",
            c"glGetBufferPointerv",
            c"glGetFragDataLocation",
            c"glGetInteger64i_v",
            c"glGetInteger64v",
            c"glGetIntegeri_v",
            c"glGetProgramBinary",
            c"glGetQueryObjectuiv",
            c"glGetQueryiv",
            c"glGetSamplerParameterfv",
            c"glGetSamplerParameteriv",
            c"glGetSynciv",
            c"glGetTransformFeedbackVarying",
            c"glGetUniformBlockIndex",
            c"glGetUniformIndices",
            c"glGetUniformuiv",
            c"glGetVertexAttribIiv",
            c"glGetVertexAttribIuiv",
            c"glIsQuery",
            c"glIsSampler",
            c"glIsSync",
            c"glIsTransformFeedback",
            c"glIsVertexArray",
            c"glPauseTransformFeedback",
            c"glProgramBinary",
            c"glProgramParameteri",
            c"glResumeTransformFeedback",
            c"glSamplerParameterf",
            c"glSamplerParameterfv",
            c"glSamplerParameteri",
            c"glSamplerParameteriv",
            c"glTransformFeedbackVaryings",
            c"glUniformBlockBinding",
            c"glUniformMatrix2x3fv",
            c"glUniformMatrix2x4fv",
            c"glUniformMatrix3x2fv",
            c"glUniformMatrix3x4fv",
            c"glUniformMatrix4x2fv",
            c"glUniformMatrix4x3fv",
            c"glVertexAttribI4i",
            c"glVertexAttribI4iv",
            c"glVertexAttribI4ui",
            c"glVertexAttribI4uiv",
            c"glWaitSync",
        ] {
            assert!(proc_addr(name).is_some(), "{name:?}");
        }
    }

    #[test]
    fn get_proc_address_returns_the_function_itself() {
        /* eglGetProcAddress writes the shared EGL error status: hold the lock. */
        let _serial = TestGl::new(8, 8);
        assert_eq!(proc_addr(c"eglGetError"), Some(eglGetError as *const () as usize));
        assert_eq!(proc_addr(c"eglSwapBuffers"), Some(eglSwapBuffers as *const () as usize));
        assert_eq!(proc_addr(c"glReadPixels"), Some(crate::gl::framebuffer::glReadPixels as *const () as usize));
        assert_eq!(proc_addr(c"glClear"), Some(crate::gl::state::glClear as *const () as usize));
        assert_ne!(proc_addr(c"glClear"), proc_addr(c"glClearColor"));
    }

    #[test]
    fn get_proc_address_resolves_the_vao_calls_under_both_their_names() {
        /* eglGetProcAddress writes the shared EGL error status: hold the lock. */
        let _serial = TestGl::new(8, 8);
        /* GL_OES_vertex_array_object became core in ES 3.0 under names
         * without the suffix: the same objects and the same calls, so the
         * core entry point forwards to the OES one (gl/vao.rs) */
        for (core, oes) in [
            (c"glGenVertexArrays", c"glGenVertexArraysOES"),
            (c"glBindVertexArray", c"glBindVertexArrayOES"),
            (c"glDeleteVertexArrays", c"glDeleteVertexArraysOES"),
            (c"glIsVertexArray", c"glIsVertexArrayOES"),
        ] {
            assert!(proc_addr(core).is_some(), "{core:?}");
            assert!(proc_addr(oes).is_some(), "{oes:?}");
        }
        assert_eq!(proc_addr(c"glBindVertexArray"), Some(crate::gl::vao::glBindVertexArray as *const () as usize));
        assert_ne!(proc_addr(c"glGenVertexArrays"), proc_addr(c"glBindVertexArray"));
    }

    #[test]
    fn get_proc_address_resolves_the_mapbuffer_calls() {
        /* eglGetProcAddress writes the shared EGL error status: hold the lock. */
        let _serial = TestGl::new(8, 8);
        /* glad loads these only through eglGetProcAddress, and only when
         * GL_OES_mapbuffer is claimed (glmark2's buffer:update-method=map) */
        assert_eq!(proc_addr(c"glMapBufferOES"), Some(crate::gl::buffer::glMapBufferOES as *const () as usize));
        assert_eq!(proc_addr(c"glUnmapBufferOES"), Some(crate::gl::buffer::glUnmapBufferOES as *const () as usize));
        assert_eq!(
            proc_addr(c"glGetBufferPointervOES"),
            Some(crate::gl::buffer::glGetBufferPointervOES as *const () as usize)
        );
    }

    #[test]
    fn get_proc_address_resolves_the_debug_marker_calls() {
        /* eglGetProcAddress writes the shared EGL error status: hold the lock. */
        let _serial = TestGl::new(8, 8);
        use crate::gl::state::{glInsertEventMarkerEXT, glPopGroupMarkerEXT, glPushGroupMarkerEXT};
        assert_eq!(proc_addr(c"glInsertEventMarkerEXT"), Some(glInsertEventMarkerEXT as *const () as usize));
        assert_eq!(proc_addr(c"glPushGroupMarkerEXT"), Some(glPushGroupMarkerEXT as *const () as usize));
        assert_eq!(proc_addr(c"glPopGroupMarkerEXT"), Some(glPopGroupMarkerEXT as *const () as usize));
    }

    #[test]
    fn get_proc_address_unknown_names_are_null() {
        /* The guard, although this asks only about a static table:
         * eglGetProcAddress clears the error word like every entry point, so
         * without it this test wipes the error another EGL test has just set
         * and is about to read. That is one lost race per few thousand runs,
         * reported against the other test. (The OSAL lock does not cover it:
         * serialising each entry point does not make another test's set and
         * read atomic -- only a per-thread error status would, EGL 1.5 3.1,
         * which this library does not have yet.) */
        let _gl = TestGl::new(8, 8);
        for name in [c"", c"glClea", c"glClearX", c"glclear", c"glBegin", c"eglFoo", c" glClear"] {
            assert!(proc_addr(name).is_none(), "{name:?}");
        }
        // SAFETY: null is tolerated.
        assert!(unsafe { eglGetProcAddress(core::ptr::null()) }.is_none());
    }

    /* ---- the kick model and the cache across EGL calls ------------------------------------------- */

    fn clear_frame() {
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.0, 1.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
        }
    }

    #[test]
    fn swap_buffers_leaves_the_new_frame_running_and_completes_the_one_before() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (s, _c) = egl.current(4, 4);
        // SAFETY: plain EGL calls.
        unsafe {
            clear_frame();
            assert_eq!(eglSwapBuffers(egl.dpy, s), TRUE);
            assert_eq!(gl.submitted(), 1);
            assert!(gl.kick_pending(1), "the frame just submitted keeps running");
            clear_frame();
            assert_eq!(eglSwapBuffers(egl.dpy, s), TRUE);
            assert!(!gl.kick_pending(1), "the previous frame is done before the swap returns");
            assert!(gl.kick_pending(2));
            assert_eq!(eglWaitGL(), TRUE);
            assert!(!gl.kick_pending(2));
        }
        assert_eq!(gl.draws().len(), 2);
    }

    #[test]
    fn swap_buffers_reports_a_failing_submit_or_wait_as_context_lost() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (s, _c) = egl.current(4, 4);
        // SAFETY: plain EGL calls.
        unsafe {
            gl.set_flush_result(-5);
            assert_eq!(eglSwapBuffers(egl.dpy, s), FALSE);
            assert_eq!(egl_error(), EGL_CONTEXT_LOST);
            gl.set_flush_result(0);
            gl.set_finish_result(-5);
            assert_eq!(eglSwapBuffers(egl.dpy, s), FALSE);
            assert_eq!(egl_error(), EGL_CONTEXT_LOST);
            gl.set_finish_result(0);
            assert_eq!(eglSwapBuffers(egl.dpy, s), TRUE);
        }
    }

    #[test]
    fn a_fence_follows_its_kick() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        egl.current(4, 4);
        clear_frame();
        let sync = fence(egl.dpy);
        assert!(!sync.is_null());
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglClientWaitSync(egl.dpy, sync, 0, 0), EGL_TIMEOUT_EXPIRED, "kick 1 is running");
            gl.complete_kicks(1);
            assert_eq!(eglClientWaitSync(egl.dpy, sync, 0, 0), EGL_CONDITION_SATISFIED);
            /* a fence with nothing recorded names the last submitted kick */
            let idle = fence(egl.dpy);
            assert_eq!(eglClientWaitSync(egl.dpy, idle, 0, 0), EGL_CONDITION_SATISFIED);
            /* a synchronous GPU: every kick is done when submitted */
            gl.set_kick_lag(Some(0));
            clear_frame();
            let done = fence(egl.dpy);
            assert_eq!(eglClientWaitSync(egl.dpy, done, 0, 0), EGL_CONDITION_SATISFIED);
            /* a lagging one: the kick before the last completes on the next submit */
            gl.set_kick_lag(Some(1));
            clear_frame();
            let lagging = fence(egl.dpy);
            assert_eq!(eglClientWaitSync(egl.dpy, lagging, 0, 0), EGL_TIMEOUT_EXPIRED);
            clear_frame();
            fence(egl.dpy);
            assert_eq!(eglClientWaitSync(egl.dpy, lagging, 0, 0), EGL_CONDITION_SATISFIED);
        }
    }

    #[test]
    fn switching_contexts_completes_the_frame_before_the_other_context_reads_it() {
        let gl = TestGl::new(8, 8);
        let mut egl = Egl::new(&gl);
        egl.free_testgl_context();
        let s = egl.pbuffer(CONFIG_1X, 2, 2);
        let (a, b) = (egl.context(CONFIG_1X), egl.context(CONFIG_1X));
        assert_eq!(egl.make_current(s, a), TRUE);
        clear_frame();
        assert_eq!(egl.make_current(s, b), TRUE);
        assert!(!gl.kick_pending(1), "the contexts share the frame sets: A's frame is done");
        let gpu = surf(s).mem.gpu;
        gl.gpu_render(gpu, &[0x42; 16]);
        let mut px = [0u8; 4];
        // SAFETY: `px` holds one RGBA pixel.
        unsafe {
            crate::gl::framebuffer::glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void)
        };
        assert_eq!(px, [0x42; 4]);
        /* the pbuffer's clear at creation, then the read-back's invalidate */
        assert_eq!(gl.cache_ops(), [CacheOp::Flush { gpu, off: 0, len: 16 }, CacheOp::Invd { gpu, off: 0, len: 16 }]);
        assert_eq!(gl_error(), GL_NO_ERROR);
    }

    #[test]
    fn a_window_surface_is_a_coherent_mapping_the_draw_may_target() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let _ops = OpsSwap::new(|o| o.map_phys = Some(t_map_phys));
        let mut pixels = std::vec![0u8; 4 * 4 * 4];
        let mut win = window(GLES_NATIVE_FORMAT_XRGB8888, 4, 4);
        win.phys = pixels.as_mut_ptr() as u64;
        let s = window_surface(egl.dpy, CONFIG_1X, &mut win);
        assert!(!s.is_null(), "0x{:x}", egl_error());
        let c = egl.context(CONFIG_1X);
        assert_eq!(egl.make_current(s, c), TRUE);
        clear_frame();
        // SAFETY: plain EGL call.
        assert_eq!(unsafe { eglSwapBuffers(egl.dpy, s) }, TRUE);
        let d = gl.draws();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].draw.target_gpu, surf(s).mem.gpu);
        /* the mock resolved the target to the mapping (else "not GPU memory") */
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(egl.release(), TRUE);
        drop(pixels);
    }

    /* ---- regression tests for the round-2 fixes ------------------------------------------ */

    #[test]
    fn destroying_an_image_keeps_the_storage_a_renderbuffer_still_renders_into() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let tex = texture(4, 4);
        let gpu = context::current().unwrap().objs().textures[tex as usize].mem.gpu;
        let img = image_from_texture(egl.dpy, c, tex);
        assert!(!img.is_null());
        let mut rb: GLuint = 0;
        // SAFETY: room for one name; `img` is a live image.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glEGLImageTargetRenderbufferStorageOES(GL_RENDERBUFFER, img);
        }
        assert_eq!(gl_error(), GL_NO_ERROR);
        gl.mem_events();
        // SAFETY: plain EGL / GL calls.
        unsafe {
            glDeleteTextures(1, &tex);
            assert_eq!(eglDestroyImage(egl.dpy, img), TRUE);
            assert!(gl.mem_events().is_empty(), "the renderbuffer still renders into it");
            /* it still is a render target: the draw path must find memory */
            let mut fb: GLuint = 0;
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            glClearColor(1.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            crate::gl::state::glFinish();
            assert_eq!(gl_error(), GL_NO_ERROR);
            /* the last user gone: now it is freed */
            glDeleteRenderbuffers(1, &rb);
        }
        assert_eq!(gl.mem_events(), [MemEvent::Free { gpu }]);
    }

    #[test]
    fn the_image_of_a_texture_outlives_it_and_is_freed_once() {
        /* gl_test case 18: context A makes an image of its texture,
         * context B samples it through a sibling, A deletes the texture,
         * the image is destroyed, both contexts are destroyed. Nothing may
         * be freed twice (the mock reports a double free) and nothing may
         * be used after the free. */
        let gl = TestGl::new(8, 8);
        let mut egl = Egl::new(&gl);
        egl.free_testgl_context();
        let _ops = OpsSwap::new(|o| o.map_phys = Some(t_map_phys));
        let (pa, ca) = (egl.pbuffer(CONFIG_1X, 4, 4), egl.context(CONFIG_1X));
        let (pb, cb) = (egl.pbuffer(CONFIG_1X, 4, 4), egl.context(CONFIG_1X));
        assert_eq!(egl.make_current(pa, ca), TRUE);
        let tex_a = texture(4, 4);
        let gpu = context::current().unwrap().objs().textures[tex_a as usize].mem.gpu;
        let img_tex = image_from_texture(egl.dpy, ca, tex_a);
        assert!(!img_tex.is_null());

        /* an image of a native buffer (a camera frame in an application) */
        let mut pixels = std::vec![0u8; 4 * 4 * 4];
        let mut nb = window(GLES_NATIVE_FORMAT_RGBA8888, 4, 4);
        nb.phys = pixels.as_mut_ptr() as u64;
        // SAFETY: `nb` is a valid native buffer description.
        let img_nb = unsafe {
            eglCreateImage(
                egl.dpy,
                NO_CONTEXT,
                EGL_NATIVE_PIXMAP_KHR as EGLenum,
                &mut nb as *mut GlesNativeWindow as EGLClientBuffer,
                core::ptr::null(),
            )
        };
        assert!(!img_nb.is_null(), "0x{:x}", egl_error());

        assert_eq!(egl.make_current(pb, cb), TRUE);
        let tex_b = texture(1, 1);
        let tex_n = texture(1, 1);
        // SAFETY: both images are live; the textures are bound in turn.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, tex_b);
            glEGLImageTargetTexture2DOES(GL_TEXTURE_2D, img_tex);
            glBindTexture(GL_TEXTURE_2D, tex_n);
            glEGLImageTargetTexture2DOES(GL_TEXTURE_2D, img_nb);
        }
        assert_eq!(gl_error(), GL_NO_ERROR);

        /* the image outlives the texture that created it */
        assert_eq!(egl.make_current(pa, ca), TRUE);
        // SAFETY: one valid name.
        unsafe { glDeleteTextures(1, &tex_a) };
        gl.mem_events();
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglDestroyImage(egl.dpy, img_tex), TRUE);
            assert_eq!(eglDestroyImage(egl.dpy, img_nb), TRUE);
        }
        assert!(gl.mem_events().is_empty(), "context B's textures still use both");
        assert_eq!(egl.release(), TRUE);
        // SAFETY: plain EGL calls.
        unsafe {
            assert_eq!(eglDestroyContext(egl.dpy, ca), TRUE);
            assert_eq!(eglDestroyContext(egl.dpy, cb), TRUE);
        }
        /* the texture's pool storage is freed exactly once, the
         * application's pages never */
        assert_eq!(gl.mem_events(), [MemEvent::Free { gpu }]);
        assert!(gl.take_faults().is_empty(), "no double free, no use after free");
    }

    #[test]
    fn an_odd_width_pbuffer_holds_the_rows_the_gpu_writes() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let s = egl.pbuffer(CONFIG_1X, 61, 35);
        assert!(!s.is_null());
        assert_eq!(surf(s).mem.size, 62 * 35 * 4, "tex_pitch(61) = 62 pixels a row");
        assert!(gl.gpu_bytes(surf(s).mem.gpu, 16).iter().all(|&b| b == 0), "cleared pool memory");
        let c = egl.context(CONFIG_1X);
        let mut egl = egl;
        egl.free_testgl_context();
        assert_eq!(egl.make_current(s, c), TRUE);
        /* glReadPixels walks the same pitch: with a tight allocation the
         * mock reported a read past the allocation */
        let mut px = std::vec![0u8; 61 * 35 * 4];
        // SAFETY: `px` holds the rectangle asked for.
        unsafe {
            crate::gl::framebuffer::glReadPixels(
                0,
                0,
                61,
                35,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                px.as_mut_ptr() as *mut c_void,
            )
        };
        assert_eq!(gl_error(), GL_NO_ERROR);
        assert!(gl.take_faults().is_empty(), "nothing outside the allocation");
    }

    #[test]
    fn a_window_surface_needs_rows_at_the_pair_pitch() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let _ops = OpsSwap::new(|o| o.map_phys = Some(t_map_phys));
        let mut packed = std::vec![0u8; 5 * 4 * 4];
        let mut win = window(GLES_NATIVE_FORMAT_RGBA8888, 5, 4);
        win.phys = packed.as_mut_ptr() as u64;
        assert_eq!(win.stride_bytes, 20);
        assert!(window_surface(egl.dpy, CONFIG_1X, &mut win).is_null(), "packed odd-width rows");
        assert_eq!(egl_error(), EGL_BAD_NATIVE_WINDOW);
        /* the same window with the pad pixel in every row is accepted */
        let mut padded = std::vec![0u8; 6 * 4 * 4];
        win.phys = padded.as_mut_ptr() as u64;
        win.stride_bytes = 24;
        let s = window_surface(egl.dpy, CONFIG_1X, &mut win);
        assert!(!s.is_null(), "0x{:x}", egl_error());
        assert_eq!((surf(s).width, surf(s).height, surf(s).mem.size), (5, 4, 6 * 4 * 4));
        let _ = gl.error();
    }

    #[test]
    fn an_image_of_a_texture_the_gpu_cannot_share_is_refused() {
        let gl = TestGl::new(8, 8);
        let egl = Egl::new(&gl);
        let (_s, c) = egl.current(4, 4);
        let mut names = [0 as GLuint; 3];
        // SAFETY: room for three names; the block data is one 4x4 ETC2 block.
        unsafe {
            glGenTextures(3, names.as_mut_ptr());
            glBindTexture(GL_TEXTURE_2D, names[0]);
            crate::gl::texture::glCompressedTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_COMPRESSED_RGB8_ETC2,
                4,
                4,
                0,
                8,
                [0u8; 8].as_ptr() as *const c_void,
            );
            glBindTexture(GL_TEXTURE_2D, names[1]);
            crate::gl::texture::glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA16F, 4, 4);
            glBindTexture(GL_TEXTURE_CUBE_MAP, names[2]);
            crate::gl::texture::glTexStorage2D(GL_TEXTURE_CUBE_MAP, 1, GL_RGBA8, 4, 4);
        }
        assert_eq!(gl_error(), GL_NO_ERROR);
        for (what, name) in ["compressed", "float", "cube map"].iter().zip(names) {
            assert!(image_from_texture(egl.dpy, c, name).is_null(), "{what}");
            assert_eq!(egl_error(), EGL_BAD_PARAMETER, "{what}");
        }
        /* and a handle that is no texture name at all */
        // SAFETY: the client buffer of EGL_GL_TEXTURE_2D is the texture name.
        let img = unsafe {
            eglCreateImage(egl.dpy, c, EGL_GL_TEXTURE_2D as EGLenum, usize::MAX as EGLClientBuffer, core::ptr::null())
        };
        assert!(img.is_null());
        assert_eq!(egl_error(), EGL_BAD_PARAMETER);
    }
}
