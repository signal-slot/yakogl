// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! dEQP-GLES2 `multisample.constancy_*` on the PowerVR, at the program level.
//!
//! The AM62P's 4x image failed `constancy_alpha_to_coverage` and
//! `constancy_both_inverted` at `32778b57` and passed them at `d60b9d76`.
//! The cause was the frame, not the program: 1,024 draws of 768 PDS bytes
//! each overflowed the 768 KiB PDS arena, the 1,015th draw opened a second
//! kick, and its load quad wrote one resolved colour to all four samples
//! (fixed by a4f64950; gl_test 198 a2cconst; tests/usc_limits.rs
//! `deqps_multisample_shader_keeps_samples_by_its_alpha_alone`). The host
//! diagnosis that ruled the program out is the record
//! tools/cts/log/investigations/es2-pvr-ms4-constancy.md.
//!
//! A constancy case draws a green quad and then a red quad of the same
//! geometry, with the same alpha and the same glSampleCoverage; the green
//! must be fully overwritten. So the set of samples a draw keeps must be a
//! function of (alpha, sample mask, alpha-to-coverage bit) alone, and never
//! of the quad's colour. Here that is checked on the program the draw runs:
//! the case's fragment shader (`gl_FragColor = v_color`) compiled for the
//! sample state (`LodKey::sample_kill`), with and without the colour mask
//! (true, true, true, false) the alpha-to-coverage cases set (a blend variant
//! on the PowerVR, which has no per-channel write mask), run in the byte model
//! once per sample with the control word `fs_meta_word` writes.
//!
//! These tests pin what the program does right, beyond usc_limits.rs's
//! colour-independence check: the kept set is exactly ES 2.0's for every
//! sample coverage mask, with and without alpha to coverage, on both ways of
//! building the program, and an alpha-masked kept sample keeps the pixel's
//! alpha.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es2_pvr_ms4_constancy
//! ```

#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr"))]

use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::{
    set_compile_lod_key, shc_blend_key_mask_off, LodKey, ShcShader, LOD_KEY_SAMPLE_KILL, SHC_BLEND_KEY_REPLACE,
    SHC_FS_META_DEFAULT, SHC_NO_META,
};

mod usc_exec;
use usc_exec::{cov_model_of, run_binary, sentinel_inputs, Inputs};

/// MultisampleCase::init's fragment shader (es2fMultisampleTests.cpp:326).
const DEQP_FS: &str = "varying mediump vec4 v_color;\nvoid main (void)\n{\n\tgl_FragColor = v_color;\n}\n";

/// `PVR_FS_META_SAMPLE_MASK` (bits 9..24) and `PVR_FS_META_ALPHA_TO_COVERAGE`
/// (bit 25), as src/pvr/backend.rs `fs_meta_word` places them.
const META_MASK_SHIFT: u32 = 9;
const META_MASK_BITS: u32 = 0xffff << META_MASK_SHIFT;
const META_A2C: u32 = 1 << 25;

/// `fs_meta_word(4, d)` for a 4x draw.
fn meta_word(mask: u32, a2c: bool) -> u32 {
    (SHC_FS_META_DEFAULT & !META_MASK_BITS & !META_A2C)
        | (mask & 0xf) << META_MASK_SHIFT
        | if a2c { META_A2C } else { 0 }
}

fn scratch_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static LENT: std::sync::Once = std::sync::Once::new();
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    LENT.call_once(|| {
        let region: &'static mut [u8] = Vec::leak(vec![0u8; 4 * store::SCRATCH_BYTES]);
        // SAFETY: a leaked region used by this binary only, under `LOCK`.
        assert_eq!(unsafe { store::shc_online_lend_scratch(region.as_mut_ptr(), region.len()) }, 0);
    });
    guard
}

/// What the program did for one sample: kept or killed, and the colour word
/// it wrote when kept.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct Sample {
    kept: bool,
    colour: u32,
}

/// Where the program comes from: `Chain` is `compile_fragment_for` with the
/// compile's sample-kill key set; `Product` is what a draw asks of the
/// back end (`ShcBackend::compile`, then `sampler_variant` with
/// `LOD_KEY_SAMPLE_KILL`, then `blend_variant`: src/gl/draw.rs
/// `draw_lod_key`, the blend variants built on a lod variant).
#[derive(Clone, Copy, Debug)]
enum Path {
    Chain,
    Product,
}

/// The sample-kill program with blend key `key` as a record, built in the
/// keep arena (`Chain`) or the product's cache (`Product`).
fn record(path: Path, key: u32) -> ShcShader {
    match path {
        Path::Chain => store::with_scratch(|a| {
            let text = a.alloc_str(DEQP_FS).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
            let old = set_compile_lod_key(LodKey { sample_kill: true, ..LodKey::NONE });
            let rec = yakogl::shaderc::online::powervr::chain::compile_fragment_for(&l.module, &l.reflection, a, key);
            set_compile_lod_key(old);
            rec.unwrap_or_else(|e| panic!("compile for the 4x sample state: {e:?}"))
        })
        .unwrap(),
        Path::Product => {
            let b = &yakogl::shaderc::online::backend::SHC_ONLINE;
            let src = std::ffi::CString::new(DEQP_FS).unwrap();
            let (mut base, mut kill, mut out) = (ShcShader::ZERO, ShcShader::ZERO, ShcShader::ZERO);
            let n = core::ptr::null_mut();
            // SAFETY: a NUL-terminated source, valid outs, no log; each call's
            // input is the previous call's output.
            unsafe {
                let rc = (b.compile.unwrap())(yakogl::shaderc::SHC_STAGE_FRAGMENT, src.as_ptr(), &mut base, n, 0);
                assert_eq!(rc, 0, "compile");
                let rc = (b.sampler_variant.unwrap())(&base, LOD_KEY_SAMPLE_KILL, &mut kill, n, 0);
                assert_eq!(rc, 0, "the sample-kill variant");
                let rc = (b.blend_variant.unwrap())(&kill, key, &mut out, n, 0);
                assert_eq!(rc, 0, "the blend variant of the sample-kill variant");
            }
            out
        }
    }
}

/// The case's shader for the 4x sample state with blend key `key`, run for
/// every sample of one pixel: `rgba` is v_color, `dst` the pixel's colour
/// already in the target.
fn run_samples(path: Path, key: u32, rgba: [f32; 4], dst: u32, mask: u32, a2c: bool) -> [Sample; 4] {
    let _scratch = scratch_lock();
    /* the chain path's code lives in a keep transaction; the product's in its cache */
    let _keep = matches!(path, Path::Chain).then(|| store::begin_keep().expect("keep"));
    let rec = record(path, key);
    assert_eq!(rec.uses_discard, 1, "{path:?}: the sample-kill epilogue");
    assert_ne!(rec.meta_off, SHC_NO_META, "{path:?}: and its control word");
    // SAFETY: the record's code lives in the keep arena or the cache, `usc_dw` dwords.
    let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
    let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
    let mut out = [Sample { kept: false, colour: 0 }; 4];
    for (s, o) in out.iter_mut().enumerate() {
        let mut inp: Inputs = sentinel_inputs(1.0e30);
        let mut at = 4usize;
        for v in &rec.varyings[..rec.num_varyings as usize] {
            for (c, x) in rgba.iter().enumerate().take(v.components as usize) {
                inp.coeff[at + 4 * c] = x.to_bits();
                inp.coeff[at + 4 * c + 2] = x.to_bits();
            }
            at += 4 * v.components as usize;
        }
        inp.shared[rec.meta_off as usize] = meta_word(mask, a2c);
        inp.samples = 4;
        inp.samp_num = s as u32;
        inp.savmsk = 0xf; /* the quad's interior: the ISP covers every sample */
        inp.pixout = dst;
        let r = run_binary(&image, &inp, true).expect("runs");
        *o = Sample { kept: r.contains_key(&usc_exec::FEEDBACK), colour: r.get(&0).copied().unwrap_or(0) };
    }
    out
}

/// The alphas the case's vertices carry: `col / 15` for the 16 columns.
fn alphas() -> impl Iterator<Item = f32> {
    (0..16).map(|c| c as f32 / 15.0)
}

fn key_for(masked_alpha: bool) -> u32 {
    if masked_alpha {
        SHC_BLEND_KEY_REPLACE | shc_blend_key_mask_off(0b1000)
    } else {
        SHC_BLEND_KEY_REPLACE
    }
}

/// Both ways of getting the program, with and without the alpha write mask.
const PATHS: [(Path, bool); 4] =
    [(Path::Chain, false), (Path::Chain, true), (Path::Product, false), (Path::Product, true)];

fn kept_set(s: &[Sample; 4]) -> u32 {
    s.iter().enumerate().map(|(i, x)| (x.kept as u32) << i).sum()
}

/// The samples the draw must keep (ES 2.0 4.1.3): in the sample coverage
/// mask, and with alpha to coverage, in the mask `alpha` makes.
fn want_kept(alpha: f32, mask: u32, a2c: bool) -> u32 {
    let cov = if a2c { cov_model_of(alpha.to_bits(), 4) } else { 0xf };
    mask & cov & 0xf
}

/// Green over red with the same alpha and sample state: the kept set must not
/// depend on the colour. This is the constancy property itself, at the
/// program level, for every mask the cases use and every alpha of the column
/// series, with the colour mask of the alpha-to-coverage cases and without.
#[test]
fn the_kept_samples_do_not_depend_on_the_colour() {
    for (path, masked_alpha) in PATHS {
        let key = key_for(masked_alpha);
        for a2c in [false, true] {
            for mask in 0..16u32 {
                for alpha in alphas() {
                    let green = run_samples(path, key, [0.0, 1.0, 0.0, alpha], 0xff00_0000, mask, a2c);
                    let red = run_samples(path, key, [1.0, 0.0, 0.0, alpha], 0xff00_0000, mask, a2c);
                    assert_eq!(
                        kept_set(&green),
                        kept_set(&red),
                        "alpha {alpha} mask {mask:#x} a2c {a2c} colour mask alpha off {masked_alpha}"
                    );
                }
            }
        }
    }
}

/// The kept set is what ES 2.0 4.1.3 says: the program is right for every
/// state the four failing/passing cases reach, so the shader is not where the
/// two cases go wrong.
#[test]
fn the_kept_samples_are_the_coverage_mask_and_alpha_to_coverage() {
    for (path, masked_alpha) in PATHS {
        let key = key_for(masked_alpha);
        for a2c in [false, true] {
            for mask in 0..16u32 {
                for alpha in alphas() {
                    let s = run_samples(path, key, [1.0, 0.0, 0.0, alpha], 0xff00_0000, mask, a2c);
                    assert_eq!(
                        kept_set(&s),
                        want_kept(alpha, mask, a2c),
                        "alpha {alpha} mask {mask:#x} a2c {a2c} colour mask alpha off {masked_alpha}"
                    );
                }
            }
        }
    }
}

/// With the colour mask (true, true, true, false) a kept sample keeps the
/// pixel's alpha and takes the draw's red, green and blue. Red over green
/// must leave no green wherever the red is kept.
#[test]
fn a_kept_sample_with_alpha_masked_writes_rgb_and_keeps_the_pixel_alpha() {
    let path = Path::Product;
    let key = key_for(true);
    let dst = 0x8000_ff00; /* green, alpha 0x80 already in the pixel */
    let s = run_samples(path, key, [1.0, 0.0, 0.0, 1.0], dst, 0xf, true);
    for x in s {
        assert!(x.kept);
        assert_eq!(x.colour, 0x8000_00ff, "red, the pixel's alpha");
    }
}
