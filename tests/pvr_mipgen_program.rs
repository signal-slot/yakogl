// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GPU mipmap quad's fragment program (tools/shaders/internal/es/
//! mipgen.frag, our compiler's row `PVR_INTERNAL_MIPGEN` of
//! src/pvr/internal.rs) in the byte model (tests/usc_exec): the program the
//! PowerVR backend draws level n of a chain with (src/pvr/backend.rs
//! `pvr_generate_mipmap`), sampling level n-1 at the centre of each of its
//! texels.
//!
//! At pixel (x, y) of a level `w` x `h`, with `u_scale` = (1 / w, 1 / h) in
//! its uniform block (sh20..21) and gl_FragCoord.y's pair (1, 0) where its
//! record says (sh22..23), the program must sample the source at
//! ((x + 0.5) / w, (y + 0.5) / h). The model's texture is 64 texels a side
//! and a sample reads the texel its coordinate falls in, so that is texel
//! (64 (x + 0.5) / w, 64 (y + 0.5) / h): the texel the frame load
//! (fbload.frag, `texelFetch` at the pixel's own integer coordinate) reads
//! at that pixel. The two programs run on the same descriptor and must
//! write the same output registers. The frame load's reading is pinned
//! against Mesa's program (tests/internal_programs.rs); this program has no
//! Mesa twin, so it is pinned against the frame load's.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_mipgen_program
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online"))]

mod usc_exec;

use std::collections::BTreeMap;
use usc_exec::{run_binary, Inputs};
use yakogl::pvr::internal::{PVR_INTERNAL_FBLOAD, PVR_INTERNAL_MIPGEN, PVR_INTERNAL_TABLE};
use yakogl::shaderc::ShcShader;

fn row(i: usize) -> &'static ShcShader {
    &PVR_INTERNAL_TABLE[i].shader
}

fn bytes(sh: &ShcShader) -> Vec<u8> {
    // SAFETY: a row of the generated table points at its `[u32; usc_dw]` static.
    let w = unsafe { std::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
    w.iter().flat_map(|d| d.to_le_bytes()).collect()
}

/// The inputs of one fragment at pixel (`x`, `y`): the pixel's centre in
/// gl_FragCoord's registers, one sample, the shared file of `seed` (the
/// descriptor at sh0..19 included).
fn at(seed: u32, x: u32, y: u32) -> Inputs {
    let mut inp = Inputs::new(seed);
    inp.samples = 1;
    let xp = (x as f32 + 0.5).to_bits();
    let yp = (y as f32 + 0.5).to_bits();
    inp.frag_xy = [xp, xp, yp, yp];
    inp
}

#[test]
fn the_record_is_the_frame_loads_shape() {
    let sh = row(PVR_INTERNAL_MIPGEN);
    assert_eq!(sh.uniform_dwords, 2, "u_scale, the block");
    assert_eq!(sh.fragcoord_ytf_off1, 23, "gl_FragCoord.y's pair straight after the block");
    assert_eq!(sh.num_outputs, 1);
    assert_eq!((sh.uses_discard, sh.uses_fbfetch, sh.num_varyings), (0, 0, 0));
    let fb = row(PVR_INTERNAL_FBLOAD);
    assert_eq!(
        (fb.uniform_dwords, fb.fragcoord_ytf_off1),
        (2, 23),
        "the frame load's, which the backend binds the same way"
    );
}

/// Levels of 8x8, 32x16 and 4x64 (every size a divisor of the model's 64,
/// so the frame load's integer coordinate lands on the same texel), at
/// pixels across each, with several descriptors: the sample is the texel
/// under the pixel's centre, scaled by u_scale.
#[test]
fn the_mipmap_quad_samples_the_source_at_the_centre_of_its_own_texel() {
    let mip = bytes(row(PVR_INTERNAL_MIPGEN));
    let fb = bytes(row(PVR_INTERNAL_FBLOAD));
    let mut runs = 0;
    for (w, h) in [(8u32, 8u32), (32, 16), (4, 64)] {
        for seed in [1u32, 77] {
            for (x, y) in [(0u32, 0u32), (w - 1, 0), (0, h - 1), (w - 1, h - 1), (w / 2, h / 3), (1, 2)] {
                let mut ours = at(seed, x, y);
                ours.nearest = false;
                ours.shared[20] = (1.0 / w as f32).to_bits();
                ours.shared[21] = (1.0 / h as f32).to_bits();
                ours.shared[22] = 1.0f32.to_bits();
                ours.shared[23] = 0.0f32.to_bits();
                /* the frame load at the pixel whose own texel is the sampled one */
                let (tx, ty) = ((64 * (2 * x + 1)) / (2 * w), (64 * (2 * y + 1)) / (2 * h));
                let mut load = at(seed, tx, ty);
                load.nearest = true;
                load.shared[20] = 1.0f32.to_bits();
                load.shared[21] = 0.0f32.to_bits();
                load.shared[22] = 1.0f32.to_bits();
                load.shared[23] = 0.0f32.to_bits();
                let what = format!("{w}x{h} pixel ({x}, {y}) seed {seed}");
                let want: BTreeMap<u32, u32> =
                    run_binary(&fb, &load, true).unwrap_or_else(|e| panic!("{what}: the frame load runs: {e}"));
                let got = run_binary(&mip, &ours, true).unwrap_or_else(|e| panic!("{what}: ours runs: {e}"));
                assert!(!want.is_empty(), "{what}: the frame load writes nothing, which proves nothing");
                assert_eq!(got, want, "{what}: the quad reads another texel than ({tx}, {ty}) of the 64-texel source");
                runs += 1;
            }
        }
    }
    assert_eq!(runs, 3 * 2 * 6);
}
