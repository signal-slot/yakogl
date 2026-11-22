// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `texture::state_at` (src/v3d/texture.rs) on the V3D 4.2 TMU texture state
//! (plan #382 item stateatut, issue #1261; the helper is not wired yet, item
//! basefix wires it in place of `texture::state`).
//!
//! Mesa's `v3d_setup_texture_shader_state` (v3dx_state.c:903-988; that file is
//! not in the vendored third_party/mesa-26.2.1, the line range is the one
//! docs/notes/v3d71-cube-seamless.md cites) programs `base_level` from the
//! sampler view's `u.tex.first_level` and `max_level` from `u.tex.last_level`:
//! both ABSOLUTE level numbers of the resource, `max_level` is not relative to
//! the base. So `state_at(.., first)` must give `base_level = min(first,
//! levels - 1)`, `max_level = levels - 1` whatever `first` is, and every
//! other field (address, size, swizzle, type, depth, stride) as
//! `texture::state`, for 2D, 3D, 2D array and cube. No mismatch with Mesa was
//! found, so there is no `_flip_when_fixed` pin.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_texture_state_at
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_int, c_void};
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use yakogl::backend::*;
use yakogl::v3d::ffi::{V3dTexLayout, V3dTexState};
use yakogl::v3d::texture;

/* ---- the layout code: the real C, as the other V3D tests load it ---- */

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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_texture_state_at");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let so = dir.join(format!("v3d_tex-{}.so", std::process::id()));
        /* the compiler's own temporary files next to the object */
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

/// The trampolines: the library's `extern "C"` texture functions
/// (src/v3d/ffi.rs), each handed to the real C.
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

/* The signatures are src/v3d/ffi.rs's, kept as written (see
 * tests/depth_resolve_v3d.rs: rustfmt releases wrap these differently). */
#[rustfmt::skip]
mod trampolines {
    use super::*;

    trampoline!(v3d_tex_layout(width: u32, height: u32, levels: u32, cpp: u32, l: *mut V3dTexLayout) -> bool);
    trampoline!(v3d_tex_state_from_layout(s: *mut V3dTexState, l: *const V3dTexLayout));
}

const LEVELS: u8 = 5;
const GPU: u32 = 0x10_0000;

fn bind(type_: GlesTexType, layers: u16) -> GlesTextureBind {
    GlesTextureBind {
        format: GLES_TEXFMT_RGBA8,
        type_,
        layers,
        width: 16,
        height: 16,
        stride_texels: 16,
        mip_filter: 1,
        mip_levels: LEVELS,
        ..GlesTextureBind::ZERO
    }
}

fn cases() -> Vec<(&'static str, GlesTextureBind)> {
    vec![
        ("2D", bind(GLES_TEXTYPE_2D, 1)),
        ("3D", bind(GLES_TEXTYPE_3D, 8)),
        ("2D array", bind(GLES_TEXTYPE_2D_ARRAY, 3)),
        ("cube", bind(GLES_TEXTYPE_CUBE, 6)),
    ]
}

#[test]
fn base_level_is_first_level_clamped_to_the_last_level() {
    for (what, b) in cases() {
        let l = texture::layout(&b).expect(what);
        assert_eq!(l.levels, LEVELS as u32, "{what}: levels");
        for first in 0..=8u32 {
            let s = texture::state_at(&b, &l, GPU, first);
            assert_eq!(s.base_level, first.min(LEVELS as u32 - 1), "{what}: base_level at {first}");
        }
        /* a huge first level reads the last level */
        assert_eq!(texture::state_at(&b, &l, GPU, u32::MAX).base_level, LEVELS as u32 - 1, "{what}");
    }
}

#[test]
fn max_level_is_absolute_not_base_relative() {
    for (what, b) in cases() {
        let l = texture::layout(&b).expect(what);
        for first in 0..=8u32 {
            let s = texture::state_at(&b, &l, GPU, first);
            assert_eq!(s.max_level, LEVELS as u32 - 1, "{what}: max_level at {first}");
            assert!(s.base_level <= s.max_level, "{what}: base past max at {first}");
        }
    }
}

#[test]
fn first_level_0_is_texture_state() {
    for (what, b) in cases() {
        let l = texture::layout(&b).expect(what);
        assert_eq!(texture::state_at(&b, &l, GPU, 0), texture::state(&b, &l, GPU), "{what}");
    }
}

#[test]
fn other_fields_do_not_change_with_first_level() {
    for (what, b) in cases() {
        let l = texture::layout(&b).expect(what);
        let s0 = texture::state(&b, &l, GPU);
        for first in 1..=8u32 {
            let s = texture::state_at(&b, &l, GPU, first);
            let same = V3dTexState { base_level: s0.base_level, ..s };
            assert_eq!(same, s0, "{what}: only base_level may move at {first}");
            assert_eq!(s.base, GPU + l.slices[0].offset, "{what}: address");
            assert_eq!((s.width, s.height), (16, 16), "{what}: size");
            assert_eq!(s.swizzle, s0.swizzle, "{what}: swizzle");
            assert_eq!(s.type_, s0.type_, "{what}: type");
        }
    }
}

#[test]
fn a_single_level_texture_stays_at_base_0() {
    for (what, mut b) in cases() {
        b.mip_levels = 1;
        let l = texture::layout(&b).expect(what);
        let s = texture::state_at(&b, &l, GPU, 3);
        assert_eq!((s.base_level, s.max_level), (0, 0), "{what}");
    }
}
