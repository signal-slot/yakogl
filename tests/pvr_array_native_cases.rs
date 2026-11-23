// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The shapes of three dEQP-GLES3 2D-array groups on the PowerVR host model,
//! each with the native 2D array on (`pvr_set_native_array(1)`, docs/es3/
//! pvr-2darray.md) and again with it off, the stacked image (the control, and
//! the default until a board run switches it):
//!
//! - texture.filtering.2d_array: a look-up of the middle layer of a 3-layer
//!   array at a level above 0, as NEAREST and LINEAR_MIPMAP_LINEAR
//!   minification reach it (an explicit level 0, 1 and 1.5 in a fragment
//!   shader; the model has no derivatives, so the implicit level is 0);
//! - texture.shadow.2d_array: one `sampler2DArrayShadow` compare with
//!   GL_LESS (the sampler meta's op 1) against a DEPTH_COMPONENT32F array
//!   (op 1) and a DEPTH24 one (op 1 | 8, the reference clamped), NEAREST and
//!   LINEAR (the per-texel compare, meta dword 13 bit 0);
//! - texture.vertex.2d_array: `textureLod` in the vertex shader.
//!
//! The host OSAL renders nothing and the byte model holds no real image
//! (tests/usc_exec): a texture is a marker, a texel a hash of the marker,
//! the texel and the channel, a layer and a level a marker of their own. So
//! "render and read back" is our compiled bytes run in the byte model, the
//! pixel (fragment) or the varying (vertex) read back and checked against
//! the value the test computes from the model's own functions
//! (`marker_at_layer`, `marker_at_level`, `sample_model`, `shadow_model`,
//! `array_uv`) and, for the fragment shaders, the IR interpreter.
//!
//! The switch is process-wide and the compiler keeps its output in one
//! arena, so every run holds one lock and puts the default back after it.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_array_native_cases
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::ir::IoSlot;
use yakogl::shaderc::online::powervr::chain::{compile_fragment, compile_vertex};
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};

mod usc_exec;

/// One compile at a time, and the switch back at its default after it.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn with_native_array<T>(on: bool, f: impl FnOnce() -> T) -> T {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    struct Back;
    impl Drop for Back {
        fn drop(&mut self) {
            yakogl::shaderc::pvr_set_native_array(yakogl::shaderc::PVR_NATIVE_ARRAY_DEFAULT);
        }
    }
    let _back = Back;
    yakogl::shaderc::pvr_set_native_array(i32::from(on));
    f()
}

/// The layers of the stacked arrays: the 3-layer arrays of the dEQP cases.
const LAYERS: f32 = 3.0;
/// The middle layer.
const MIDDLE: f32 = 1.0;

fn marker(name: &str) -> u32 {
    usc_exec::named(7, name, 99)
}

fn name_of(n: &[core::ffi::c_char]) -> String {
    n.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

/// A uniform's dword by name: the shader's inputs.
type Val<'a> = dyn Fn(&str, u32) -> u32 + 'a;

/// A uniform's value: the test's own, the stacked layer count `LAYERS`.
fn uniform(val: &Val<'_>, n: &str, k: u32) -> u32 {
    if n.starts_with("pvr_layers_") {
        LAYERS.to_bits()
    } else {
        val(n, k)
    }
}

const HEAD: &str = "#version 300 es\nprecision highp float;\nprecision highp int;\nprecision highp sampler2DArray;\nprecision highp sampler2DArrayShadow;\n";

fn fragment(sampler: &str, body: &str) -> String {
    format!("{HEAD}uniform {sampler} s;\nuniform vec4 c;\nuniform float l;\nout vec4 o;\nvoid main() {{ {body} }}\n")
}

fn vertex(body: &str) -> String {
    format!(
        "{HEAD}uniform sampler2DArray s;\nuniform vec4 c;\nuniform float l;\nin vec4 a;\nout vec4 v;\nvoid main() {{ gl_Position = a; {body} }}\n"
    )
}

/// The fragment shader `src` run in the byte model with the shadow meta
/// `meta`: the four bytes of the pixel, asserted equal to the IR's unless
/// the interpreter has no such look-up (the stacked shadow array).
fn run_fragment(on: bool, src: &str, meta: [u32; 2], val: &Val<'_>) -> [u8; 4] {
    with_native_array(on, || {
        usc_exec::set_smp_meta(meta);
        store::with_scratch(|a| {
            let _keep = store::begin_keep().expect("keep");
            let text = a.alloc_str(src).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text)
                .unwrap_or_else(|e| panic!("front end: {}\n{src}", e.msg));
            let rec = compile_fragment(&l.module, &l.reflection, a)
                .unwrap_or_else(|e| panic!("native {on}: refused: {e:?}\n{src}"));
            // SAFETY: the record's code and uniform rows live in `keep`.
            let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
            let rows = unsafe { core::slice::from_raw_parts(rec.uniforms, rec.num_uniforms as usize) };
            let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
            assert_eq!(rec.num_varyings, 0, "the harness places no varyings");
            let no_block = yakogl::shaderc::online::reflect::NO_BLOCK;
            let mut inp = usc_exec::sentinel_inputs(1.0e30);
            let mut samplers: Vec<&yakogl::shaderc::ShcUniform> =
                rows.iter().filter(|u| u.dword_off == no_block).collect();
            samplers.sort_by_key(|u| u.binding);
            for (i, u) in samplers.iter().enumerate() {
                let m = marker(&name_of(&u.name));
                inp.shared[20 * i..20 * i + 4].copy_from_slice(&usc_exec::tex_state(m));
                inp.shared[20 * i + 4..20 * i + 7].copy_from_slice(&usc_exec::tex_meta(m));
                inp.shared[20 * i + 12..20 * i + 14].copy_from_slice(&meta);
            }
            let base = 20 * samplers.len();
            for u in rows.iter().filter(|u| u.dword_off != no_block) {
                for k in 0..u.dwords as u32 {
                    inp.shared[base + u.dword_off as usize + k as usize] = uniform(val, &name_of(&u.name), k);
                }
            }
            let ours = usc_exec::run_binary(&image, &inp, true)
                .unwrap_or_else(|e| panic!("native {on}: ours does not run: {e}\n{src}"))[&0];
            let ir_in = usc_exec::ir::IrInputs {
                input: &|_, _| 0,
                uniform: &|d| {
                    let u = l.reflection.uniforms.iter().find(|u| (u.dword_off..u.dword_off + u.dwords).contains(&d));
                    u.map_or(0, |u| uniform(val, u.name, (d - u.dword_off) as u32))
                },
                marker: &|binding| {
                    let s = l.reflection.uniforms.iter().find(|u| u.binding == binding && u.dword_off == no_block);
                    marker(s.map_or("s", |u| u.name))
                },
            };
            match usc_exec::ir::run_ir(&l.module, &ir_in) {
                Ok(want) => {
                    let (_, lanes) = want.outputs.iter().find(|(s, _)| *s == IoSlot::FragColor(0)).expect("a colour");
                    let mut px = 0u32;
                    for (c, lane) in lanes.iter().enumerate() {
                        let x = f32::from_bits(lane.expect("stored").0);
                        px |= ((x.clamp(0.0, 1.0) * 255.0).round() as u32) << (8 * c);
                    }
                    assert_eq!(ours, px, "native {on}: ours {ours:08x}, the IR {px:08x}\n{src}");
                }
                Err(e) if e.contains("is not in this interpreter") && !on => {}
                Err(e) => panic!("native {on}: the IR: {e}\n{src}"),
            }
            ours.to_le_bytes()
        })
        .unwrap()
    })
}

/// The vertex shader `src` run in the byte model: the four words of its
/// varying `v`.
fn run_vertex(on: bool, src: &str, val: &Val<'_>) -> [u32; 4] {
    with_native_array(on, || {
        store::with_scratch(|a| {
            let _keep = store::begin_keep().expect("keep");
            let text = a.alloc_str(src).unwrap();
            let l =
                lower::compile_to_ir(a, Stage::Vertex, text).unwrap_or_else(|e| panic!("front end: {}\n{src}", e.msg));
            let rec = compile_vertex(&l.module, &l.reflection, a)
                .unwrap_or_else(|e| panic!("native {on}: refused: {e:?}\n{src}"));
            // SAFETY: the record's code and uniform rows live in `keep`.
            let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
            let rows = unsafe { core::slice::from_raw_parts(rec.uniforms, rec.num_uniforms as usize) };
            let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
            let no_block = yakogl::shaderc::online::reflect::NO_BLOCK;
            let mut inp = usc_exec::sentinel_inputs(1.0e30);
            let mut samplers: Vec<&yakogl::shaderc::ShcUniform> =
                rows.iter().filter(|u| u.dword_off == no_block).collect();
            samplers.sort_by_key(|u| u.binding);
            for (i, u) in samplers.iter().enumerate() {
                let m = marker(&name_of(&u.name));
                inp.shared[20 * i..20 * i + 4].copy_from_slice(&usc_exec::tex_state(m));
                inp.shared[20 * i + 4..20 * i + 7].copy_from_slice(&usc_exec::tex_meta(m));
            }
            let base = 20 * samplers.len();
            for u in rows.iter().filter(|u| u.dword_off != no_block) {
                for k in 0..u.dwords as u32 {
                    inp.shared[base + u.dword_off as usize + k as usize] = uniform(val, &name_of(&u.name), k);
                }
            }
            let out = usc_exec::run_binary(&image, &inp, true)
                .unwrap_or_else(|e| panic!("native {on}: ours does not run: {e}\n{src}"));
            let v = rec.varyings[..rec.num_varyings as usize]
                .iter()
                .find(|v| name_of(&v.name) == "v")
                .expect("the varying v");
            [0, 1, 2, 3].map(|k| *out.get(&(u32::from(v.vtxout_off) + k)).expect("v stored"))
        })
        .unwrap()
    })
}

/// A texel's byte as the fragment output writes it.
fn byte(x: f32) -> u8 {
    (x.clamp(0.0, 1.0) * 255.0).round() as u8
}

const SAMPLER: &str = "sampler2DArray";

/// The inputs of the cases: the point of the layer (u, v), `z`, the shadow
/// reference `r` in lane 3, and level 0.
fn coord(z: f32, r: f32) -> impl Fn(&str, u32) -> u32 {
    move |n: &str, k: u32| match n {
        "c" => [0.3f32, 0.8, z, r][k as usize].to_bits(),
        "l" => 0.0f32.to_bits(),
        _ => 0,
    }
}

/// The layer a native array sample of `z` reads: `floor(z + 0.5)` clamped to
/// the image's depth.
fn native_layer(z: f32) -> u32 {
    ((z + 0.5).floor().max(0.0)).min(usc_exec::tex_depth(marker("s")) as f32) as u32
}

/// What the model reads for a native array sample of `z` at level `lod`:
/// the layer's marker, then the level's, the texel at (u, v).
fn native_texel(z: f32, lod: f32, u: f32, v: f32, k: u32) -> f32 {
    let layered = usc_exec::marker_at_layer(marker("s"), native_layer(z));
    let m = usc_exec::marker_at_level(layered, lod.to_bits());
    f32::from_bits(usc_exec::sample_model(m, u.to_bits(), v.to_bits(), k))
}

/// The model's native `s` has a middle layer, and its layers differ.
fn model_has_distinct_layers() {
    assert!(usc_exec::tex_depth(marker("s")) >= 2, "the model's `s` has no middle layer to sample");
    let at = |z: f32| native_texel(z, 0.0, 0.3, 0.8, 0);
    assert!(at(0.0) != at(MIDDLE) && at(MIDDLE) != at(2.0), "the model's layers read alike: the check would be blind");
}

/// texture.filtering.2d_array: the middle layer, at the levels NEAREST
/// (level 0) and LINEAR_MIPMAP_LINEAR (a level above 0) minification reach,
/// native: the model's texel of that layer and level, the bytes of the
/// pixel, and the IR agrees.
#[test]
fn filtering_2d_array_native_reads_the_middle_layer_at_each_level() {
    model_has_distinct_layers();
    let mut seen = std::collections::BTreeSet::new();
    for lod in [0.0f32, 1.0, 1.5] {
        let src = fragment(SAMPLER, "o = textureLod(s, c.xyz, l);");
        let val = move |n: &str, k: u32| match n {
            "l" => lod.to_bits(),
            _ => coord(MIDDLE, 0.0)(n, k),
        };
        let px = run_fragment(true, &src, [0, 0], &val);
        let want: [u8; 4] = [0, 1, 2, 3].map(|k| byte(native_texel(MIDDLE, lod, 0.3, 0.8, k)));
        assert_eq!(px, want, "native, textureLod level {lod}");
        seen.insert(px);
    }
    /* level 1 is not level 0 (1.5 floors to 1) */
    assert!(seen.len() >= 2, "levels 0 and 1 read the same texel: {seen:?}");
    /* the implicit-level form (NEAREST's plain `texture`, level 0 in the model) */
    let src = fragment(SAMPLER, "o = texture(s, c.xyz);");
    let px = run_fragment(true, &src, [0, 0], &coord(MIDDLE, 0.0));
    assert_eq!(px, [0, 1, 2, 3].map(|k| byte(native_texel(MIDDLE, 0.0, 0.3, 0.8, k))), "native, texture()");
    /* the layers are told apart through the shader */
    let at = |z: f32| run_fragment(true, &src, [0, 0], &coord(z, 0.0));
    assert!(at(0.0) != px && at(2.0) != px, "the middle layer reads as its neighbour");
}

/// The same forms on the stacked image (the control): the pixel agrees with
/// the IR, which computes the stacked arm as glsl2usc's `pvr_arrUV` does,
/// and at level 0 with the model's texel at that helper's coordinate.
#[test]
fn filtering_2d_array_stacked_agrees_with_the_ir_and_the_helper() {
    for lod in [0.0f32, 1.0, 1.5] {
        let src = fragment(SAMPLER, "o = textureLod(s, c.xyz, l);");
        let val = move |n: &str, k: u32| match n {
            "l" => lod.to_bits(),
            _ => coord(MIDDLE, 0.0)(n, k),
        };
        /* run_fragment asserts the IR's pixel */
        run_fragment(false, &src, [0, 0], &val);
    }
    let src = fragment(SAMPLER, "o = texture(s, c.xyz);");
    let px = run_fragment(false, &src, [0, 0], &coord(MIDDLE, 0.0));
    let m = marker("s");
    let c = [0.3f32, 0.8, MIDDLE].map(f32::to_bits);
    let (u, v) = usc_exec::array_uv(usc_exec::tex_state(m), LAYERS.to_bits(), c);
    let want: [u8; 4] = [0, 1, 2, 3].map(|k| byte(f32::from_bits(usc_exec::sample_model(m, u, v, k))));
    assert_eq!(px, want, "stacked, texture(): the model's texel at pvr_arrUV's coordinate");
}

/// What a shadow look-up of reference `r` at layer `z` returns on the native
/// array: the depth of that layer's texel compared with the sampler op, per
/// texel and weighted when `flags` says so.
fn shadow_expect(op: u32, flags: u32, z: f32, r: f32, u: f32, v: f32) -> f32 {
    let base = marker("s");
    let layered = usc_exec::marker_at_layer(base, native_layer(z));
    let d = f32::from_bits(usc_exec::sample_model(layered, u.to_bits(), v.to_bits(), 0));
    let taps = usc_exec::gather_model(layered, u.to_bits(), v.to_bits(), [0, 0]).map(f32::from_bits);
    let [w, h] = usc_exec::tex_size_2d(usc_exec::tex_state(base), 0);
    let weights = usc_exec::gather_weights(u, v, w, h);
    usc_exec::shadow_model(op, flags, d, r, Some((taps, weights)))
}

/// The references around the model's depths (k/1000): below all, between,
/// above all, and one past 1 that a fixed-point depth clamps.
const REFS: [f32; 5] = [0.0, 0.25, 0.5, 0.999, 1.5];

/// texture.shadow.2d_array: `texture(s, vec4(u, v, layer, ref))` with
/// GL_LESS against a float depth (op 1) and a fixed-point one (op 1 | 8),
/// NEAREST (flags 0) and LINEAR (the per-texel compare, flags 1), native.
#[test]
fn shadow_2d_array_native_compares_less_against_the_middle_layer() {
    model_has_distinct_layers();
    let src = fragment("sampler2DArrayShadow", "o = vec4(texture(s, c));");
    let mut results = std::collections::BTreeSet::new();
    for (what, op) in [("DEPTH_COMPONENT32F", 1u32), ("DEPTH24", 1 | 8)] {
        for flags in [0u32, 1] {
            for r in REFS {
                let px = run_fragment(true, &src, [op, flags], &coord(MIDDLE, r));
                let want = byte(shadow_expect(op, flags, MIDDLE, r, 0.3, 0.8));
                assert_eq!(px, [want; 4], "native {what} flags {flags} ref {r}");
                results.insert(want);
            }
        }
    }
    assert!(results.contains(&0) && results.contains(&255), "the references never straddle the depth: {results:?}");
}

/// The stacked shadow array: the control. The stacked arm may have no
/// shadow array at all (the IR interpreter has none, and glsl2usc's
/// pvr_arr has no compare): a refusal is the pinned behaviour, a program is
/// checked to be a compare result.
#[test]
fn shadow_2d_array_stacked_control() {
    let src = fragment("sampler2DArrayShadow", "o = vec4(texture(s, c));");
    let out = std::panic::catch_unwind(|| run_fragment(false, &src, [1, 0], &coord(MIDDLE, 0.5)));
    match out {
        Ok(px) => {
            assert!(px[0] == 0 || px[0] == 255, "a NEAREST compare is 0 or 1: {px:?}");
            assert_eq!(px[0], px[3], "the result is splat to every channel");
        }
        Err(e) => {
            let m = e.downcast_ref::<String>().cloned().unwrap_or_default();
            assert!(m.contains("refused"), "the stacked shadow array fails some other way: {m}");
        }
    }
}

/// texture.vertex.2d_array: `textureLod` in the vertex shader, native: the
/// middle layer at the level, the varying's four words the model's texel.
#[test]
fn vertex_2d_array_native_looks_up_the_middle_layer() {
    model_has_distinct_layers();
    let src = vertex("v = textureLod(s, c.xyz, l);");
    let mut seen = std::collections::BTreeSet::new();
    for lod in [0.0f32, 1.0, 1.5] {
        let val = move |n: &str, k: u32| match n {
            "l" => lod.to_bits(),
            _ => coord(MIDDLE, 0.0)(n, k),
        };
        let v = run_vertex(true, &src, &val);
        let want: [u32; 4] = [0, 1, 2, 3].map(|k| native_texel(MIDDLE, lod, 0.3, 0.8, k).to_bits());
        assert_eq!(v, want, "native, vertex textureLod level {lod}");
        seen.insert(v);
    }
    assert!(seen.len() >= 2, "levels 0 and 1 read the same texel: {seen:?}");
    let other = run_vertex(true, &src, &coord(2.0, 0.0));
    assert!(!seen.contains(&other), "layer 2 reads as the middle layer");
}

/// The vertex look-up on the stacked image: the model's texel at pvr_arrUV's
/// coordinate at level 0 (the level's marker for a higher one).
#[test]
fn vertex_2d_array_stacked_control() {
    let src = vertex("v = textureLod(s, c.xyz, l);");
    for lod in [0.0f32, 1.0] {
        let val = move |n: &str, k: u32| match n {
            "l" => lod.to_bits(),
            _ => coord(MIDDLE, 0.0)(n, k),
        };
        let v = run_vertex(false, &src, &val);
        let m = marker("s");
        let c = [0.3f32, 0.8, MIDDLE].map(f32::to_bits);
        let (u, w) = usc_exec::array_uv(usc_exec::tex_state(m), LAYERS.to_bits(), c);
        let texel = usc_exec::marker_at_level(m, lod.to_bits());
        let want: [u32; 4] = [0, 1, 2, 3].map(|k| usc_exec::sample_model(texel, u, w, k));
        assert_eq!(v, want, "stacked, vertex textureLod level {lod}");
    }
}

/// The default is back after a run with the switch on.
#[test]
fn the_default_is_restored() {
    with_native_array(true, || {});
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(yakogl::shaderc::native_array_enabled(), yakogl::shaderc::PVR_NATIVE_ARRAY_DEFAULT != 0);
}
