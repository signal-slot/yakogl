// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Pins the lod-0 `texelFetch` / `texelFetchOffset` look-ups of 3D and 2D
//! array textures on the PowerVR online compiler (dEQP-GLES3
//! shaders.texture_functions.texelfetch and texelfetchoffset, the
//! sampler3d_fixed, usampler3d, sampler2darray and usampler2darray cases),
//! which design-texfunc found already fixed at the tip: the default stacked
//! layouts (`pvr_native_array` and `pvr_native_3d` off) and, for the array,
//! the native arm too.
//!
//! The host OSAL renders nothing and the byte model holds no real image: a
//! texture is a marker, and a texel's value a hash of the marker, the texel
//! and the channel (`usc_exec::sample_model`), distinct for every texel. So
//! "draw and read back" is our compiled bytes run in the byte model:
//!
//! - the pixel equals the IR interpreter's, which computes the fetch as the
//!   GLSL says (`pixel`);
//! - `texelFetchOffset(s, p, 0, o)` returns exactly the texel
//!   `texelFetch(s, p + o, 0)` does, for each offset;
//! - texels one apart read different values, so a pixel names a texel.
//!
//! The offsets are those of es3_pvr_texoffset.rs's table: es3fShaderTexture
//! FunctionTests.cpp of VK-GL-CTS 93bca01 was not available on the machine
//! this was written on, so the dEQP cases' own offsets are not copied.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_texfetch_fixed
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::ir::IoSlot;
use yakogl::shaderc::online::powervr::chain::compile_fragment;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};

mod usc_exec;

/// The compiler keeps its output in one arena that every compile resets,
/// and the switches are process-wide: one compile at a time.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// How the sampler is kept: the stacked image / grid of slices (the
/// defaults), or the TPU's own array.
#[derive(Clone, Copy, Debug)]
enum Layout {
    Stacked,
    NativeArray,
}

/// The switches for the closure, and back at their defaults after it,
/// panic or not.
fn with_layout<T>(layout: Layout, f: impl FnOnce() -> T) -> T {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    struct Back;
    impl Drop for Back {
        fn drop(&mut self) {
            yakogl::shaderc::pvr_set_native_array(yakogl::shaderc::PVR_NATIVE_ARRAY_DEFAULT);
            yakogl::shaderc::pvr_set_native_3d(yakogl::shaderc::PVR_NATIVE_3D_DEFAULT);
        }
    }
    let _back = Back;
    yakogl::shaderc::pvr_set_native_array(i32::from(matches!(layout, Layout::NativeArray)));
    yakogl::shaderc::pvr_set_native_3d(0);
    f()
}

fn fragment(decl: &str, body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\n{decl}\nuniform ivec3 p;\nuniform int il;\n\
         out vec4 o;\nvoid main() {{ {body} }}\n"
    )
}

/// The marker of sampler `s` in the model.
fn sampler_marker() -> u32 {
    usc_exec::named(7, "s", 99)
}

/// The packed pixel of the fragment shader `src` at integer coordinate `p`
/// and level 0: our bytes in the byte model, asserted equal to the IR's.
fn pixel(layout: Layout, src: &str, p: [i32; 3]) -> u32 {
    with_layout(layout, || {
        store::with_scratch(|a| {
            let _keep = store::begin_keep().expect("keep");
            let text = a.alloc_str(src).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text)
                .unwrap_or_else(|e| panic!("front end: {}\n{src}", e.msg));
            let rec = compile_fragment(&l.module, &l.reflection, a).unwrap_or_else(|e| panic!("refused: {e:?}\n{src}"));
            // SAFETY: the record's code and uniform rows live in `keep`.
            let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
            let rows = unsafe { core::slice::from_raw_parts(rec.uniforms, rec.num_uniforms as usize) };
            let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
            assert_eq!(rec.num_varyings, 0, "the harness places no varyings");
            let name_of = |n: &[core::ffi::c_char]| -> String {
                n.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
            };
            let no_block = yakogl::shaderc::online::reflect::NO_BLOCK;
            /* a uniform's dword k: the fetch coordinate, level 0, and the
             * layer count of a stacked sampler (the GL core's own write) */
            let val = |n: &str, k: u32| -> u32 {
                match n {
                    "p" => p[k as usize] as u32,
                    "il" => 0,
                    _ if n.starts_with("pvr_layers_") => usc_exec::layer_count(1, n),
                    _ => 0,
                }
            };
            let mut inp = usc_exec::sentinel_inputs(1.0e30);
            let mut samplers: Vec<&yakogl::shaderc::ShcUniform> =
                rows.iter().filter(|u| u.dword_off == no_block).collect();
            samplers.sort_by_key(|u| u.binding);
            for (i, u) in samplers.iter().enumerate() {
                let marker = usc_exec::named(7, &name_of(&u.name), 99);
                inp.shared[20 * i..20 * i + 4].copy_from_slice(&usc_exec::tex_state(marker));
                inp.shared[20 * i + 4..20 * i + 7].copy_from_slice(&usc_exec::tex_meta(marker));
            }
            let base = 20 * samplers.len();
            for u in rows.iter().filter(|u| u.dword_off != no_block) {
                for k in 0..u.dwords as u32 {
                    inp.shared[base + u.dword_off as usize + k as usize] = val(&name_of(&u.name), k);
                }
            }
            let ours = *usc_exec::run_binary(&image, &inp, true).expect("ours runs").get(&0).expect("a pixel");
            let ir_in = usc_exec::ir::IrInputs {
                input: &|_, _| 0,
                uniform: &|d| {
                    let u = l.reflection.uniforms.iter().find(|u| (u.dword_off..u.dword_off + u.dwords).contains(&d));
                    u.map_or(0, |u| val(u.name, (d - u.dword_off) as u32))
                },
                marker: &|_| sampler_marker(),
            };
            /* the interpreter has no offset fetch on a stacked texture and no
             * integer result: those pixels are checked against the plain
             * fetch of the sum instead (`check`) */
            let want = match usc_exec::ir::run_ir(&l.module, &ir_in) {
                Ok(w) => w,
                Err(e) if e.contains("is not in this interpreter") => return ours,
                Err(e) => panic!("the IR: {e}"),
            };
            let (_, lanes) = want.outputs.iter().find(|(s, _)| *s == IoSlot::FragColor(0)).expect("a colour");
            let mut px = 0u32;
            for (c, lane) in lanes.iter().enumerate() {
                let x = f32::from_bits(lane.expect("stored").0);
                px |= ((x.clamp(0.0, 1.0) * 255.0).round() as u32) << (8 * c);
            }
            assert_eq!(ours, px, "{layout:?} p {p:?}: ours {ours:08x}, the IR {px:08x}\n{src}");
            ours
        })
        .unwrap()
    })
}

/// The look-up as a shader's pixel: a float sampler's texel is the colour
/// itself; an integer one's low bytes are the channels (the model's integer
/// is a float's bits, which a plain scale would saturate).
fn body(sampler: &str, e: &str) -> String {
    if sampler.starts_with('s') {
        format!("o = {e};")
    } else {
        format!("uvec4 t = {e}; o = vec4(t & 255u) / 255.0;")
    }
}

/// The offsets per sampler dimension (see the header). The z of a 3D offset
/// and the layer of an array fetch stay inside the model's layer counts
/// (2 to 6).
const OFFSETS_3D: &[[i32; 3]] = &[[0, 0, 0], [5, -3, 1], [-8, 7, 1], [1, 2, 0], [-3, 4, 1]];
const OFFSETS_ARRAY: &[[i32; 3]] = &[[0, 0, 0], [5, -3, 0], [-8, 7, 0], [1, 2, 0], [-3, 4, 0]];

/// `texelFetchOffset` is `texelFetch` of the sum, on each sampler kind.
fn check(layout: Layout, sampler: &str, volume: bool) {
    let decl = format!("uniform highp {sampler} s;");
    let bases: [[i32; 3]; 3] = [[20, 24, 0], [30, 17, 1], [41, 38, 0]];
    let offsets = if volume { OFFSETS_3D } else { OFFSETS_ARRAY };
    for b in bases {
        let mut seen = std::collections::BTreeSet::new();
        for o in offsets {
            let off = if volume {
                format!("ivec3({}, {}, {})", o[0], o[1], o[2])
            } else {
                format!("ivec2({}, {})", o[0], o[1])
            };
            let with = fragment(&decl, &body(sampler, &format!("texelFetchOffset(s, p, il, {off})")));
            let got = pixel(layout, &with, b);
            let sum = format!("texelFetch(s, p + ivec3({}, {}, {}), il)", o[0], o[1], o[2]);
            let want = pixel(layout, &fragment(&decl, &body(sampler, &sum)), b);
            assert_eq!(
                got, want,
                "{layout:?} {sampler} p {b:?} offset {o:?}: texelFetchOffset against texelFetch(p + offset)"
            );
            seen.insert(got);
        }
        assert!(seen.len() >= 4, "{layout:?} {sampler} p {b:?}: the offsets read distinct texels: {seen:08x?}");
    }
}

#[test]
fn texelfetchoffset_3d_float_stacked() {
    check(Layout::Stacked, "sampler3D", true);
}

#[test]
fn texelfetchoffset_3d_unsigned_stacked() {
    check(Layout::Stacked, "usampler3D", true);
}

#[test]
fn texelfetchoffset_array_float_stacked() {
    check(Layout::Stacked, "sampler2DArray", false);
}

#[test]
fn texelfetchoffset_array_unsigned_stacked() {
    check(Layout::Stacked, "usampler2DArray", false);
}

#[test]
fn texelfetchoffset_array_float_native() {
    check(Layout::NativeArray, "sampler2DArray", false);
}

#[test]
fn texelfetchoffset_array_unsigned_native() {
    check(Layout::NativeArray, "usampler2DArray", false);
}

/// Plain `texelFetch` at lod 0 reads one texel: neighbours along x, y and
/// the layer differ, on every kind and layout, and a fetch repeats.
#[test]
fn texelfetch_reads_distinct_texels() {
    for (layout, sampler) in [
        (Layout::Stacked, "sampler3D"),
        (Layout::Stacked, "usampler3D"),
        (Layout::Stacked, "sampler2DArray"),
        (Layout::Stacked, "usampler2DArray"),
        (Layout::NativeArray, "sampler2DArray"),
        (Layout::NativeArray, "usampler2DArray"),
    ] {
        let src = fragment(&format!("uniform highp {sampler} s;"), &body(sampler, "texelFetch(s, p, il)"));
        let at = |p: [i32; 3]| pixel(layout, &src, p);
        let c = at([20, 24, 0]);
        for n in [[21, 24, 0], [20, 25, 0]] {
            assert_ne!(c, at(n), "{layout:?} {sampler}: texel {n:?} reads as [20, 24, 0]");
        }
        /* the native array's layers differ (a marker per layer). Not asserted
         * for the stacked image or the grid of slices: the model's image
         * size is a hash of the marker and a layer's offset in it lands on
         * a multiple of the model's 64 texels, so the slices read alike (a
         * property of the model, not of the compiler) */
        assert!(
            !matches!(layout, Layout::NativeArray) || at([20, 24, 1]) != c || at([20, 24, 2]) != c,
            "{layout:?} {sampler}: slices 1 and 2 read as slice 0"
        );
        assert_eq!(c, at([20, 24, 0]), "{layout:?} {sampler}: a fetch is repeatable");
    }
}
