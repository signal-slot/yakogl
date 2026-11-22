// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 texture level layout (`src/v3d/tfuplan71.rs` and
//! `src/v3d/texture71.rs`) against Mesa's slice rules and the Pi 5 references.
//!
//! Run: `cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_slices`
//!
//! Rule (Mesa 26.2.1 `v3d_setup_slices`, v3d_resource.c:583-767, not
//! vendored; quoted by docs/notes/v3d-tex-layout-es3.md section 1): levels
//! are laid out from the last up to level 0, so level 0 is the LAST and
//! largest and the smaller levels sit below it. Each level is LINEARTILE (a
//! side <= one utile), UBLINEAR 1 or 2 columns (width <= 1 or 2 UIF blocks)
//! or UIF, from the level's size alone; levels >= 2 use the size rounded to a
//! power of two from level 1; level 1 is page aligned when it could be UIF
//! XOR; the tree is shifted so level 0 starts on 4 KiB. The references:
//! tools/v3d/pi5/gl_mipgen.{stdout,clif} (64x64 RGBA8, TFU job with IOC
//! 0x00086061: NUMMM 6, IIA = IOA = level 0) and gl_tex2d.clif.

#![allow(dead_code)]

#[path = "../src/v3d/format71.rs"]
mod format71;
#[path = "../src/v3d/texstate71.rs"]
mod texstate71;
#[path = "../src/v3d/texture71.rs"]
mod texture71;
#[path = "../src/v3d/tfuplan71.rs"]
mod tfuplan71;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// `name 0x...` value of a SUBMIT_TFU line.
fn tfu_field(line: &str, name: &str) -> u32 {
    let mut it = line.split_whitespace();
    while let Some(w) = it.next() {
        if w == name {
            return u32::from_str_radix(it.next().unwrap().trim_start_matches("0x"), 16).unwrap();
        }
    }
    panic!("{name} not in {line}");
}

fn tfu_line() -> String {
    read("tools/v3d/pi5/gl_mipgen.stdout")
        .lines()
        .find(|l| l.contains("SUBMIT_TFU icfg"))
        .expect("TFU line")
        .to_string()
}

/// Addresses of the `resource_0x...` BOs of a clif dump.
fn resource_bases(clif: &str) -> Vec<u32> {
    clif.lines()
        .filter_map(|l| l.split_whitespace().last())
        .filter_map(|n| n.strip_prefix("resource_0x"))
        .filter_map(|h| u32::from_str_radix(h, 16).ok())
        .collect()
}

fn tex64() -> texture71::Layout {
    texture71::layout(64, 64, 7, 4, 1).unwrap()
}

fn args(w: u32, h: u32, last: u32, cpp: u32) -> tfuplan71::Args {
    tfuplan71::Args {
        width: w,
        height: h,
        base_level: 0,
        last_level: last,
        src_level: 0,
        ttype: 6,
        cpp,
        src_tiling: tfuplan71::Tiling::UifNoXor,
        dst_tiling: tfuplan71::Tiling::UifNoXor,
        layer: 0,
        layer_count: 1,
    }
}

fn plan64() -> tfuplan71::Job {
    tfuplan71::plan(&args(64, 64, 6, 4)).unwrap().remove(0)
}

#[test]
fn reference_tfu_job() {
    let l = tfu_line();
    let ioc = tfu_field(&l, "ioc");
    assert_eq!(ioc, 0x00086061);
    let j = plan64();
    assert_eq!(ioc & 0xf0, j.ioc_mm & 0xf0, "NUMMM");
    assert_eq!(ioc & 1, j.ioc_mm & 1, "DIMTW");
    // IIA = IOA = level 0: the TFU writes the smaller levels below it
    let (iia, ioa) = (tfu_field(&l, "iia"), tfu_field(&l, "ioa"));
    assert_eq!(iia, ioa);
    // the texture BO of gl_mipgen: the resource that holds level 0
    let bases = resource_bases(&read("tools/v3d/pi5/gl_mipgen.clif"));
    let bo = bases.iter().copied().filter(|b| *b <= iia).max().unwrap();
    assert_eq!(u64::from(iia - bo), j.base_offset, "level 0 offset in the BO");
    assert_eq!(tfu_field(&l, "ios"), 0x0040_0040);
    let all = read("tools/v3d/pi5/gl_mipgen.stdout");
    assert!(all.contains("OTYPE 4") && all.contains("STRIDE 8") && all.contains("NUMMM 6"));
}

#[test]
fn rgba8_64_levels_by_mesa_rules() {
    use texture71::Tiling as T;
    let t = tex64();
    // from level 6 up, by hand: cpp 4 -> utile 4x4, UIF block 8x8, pot 64;
    // (tiling, offset before the 4 KiB shift, size)
    let want = [
        (T::LinearTile, 0u32, 64u32),    // level 6: 1x1 -> 4x4
        (T::LinearTile, 64, 64),         // level 5: 2x2 -> 4x4
        (T::LinearTile, 128, 64),        // level 4: 4x4
        (T::UbLinear1Column, 192, 256),  // level 3: 8x8
        (T::UbLinear2Column, 448, 1024), // level 2: 16x16
        (T::UifNoXor, 1472, 4096),       // level 1: 32x32
        (T::UifNoXor, 5568, 16384),      // level 0: 64x64
    ];
    let shift = 8192 - 5568;
    for (k, (tiling, off, size)) in want.iter().enumerate() {
        let lvl = 6 - k;
        let s = t.slices[lvl];
        assert_eq!((s.tiling, s.offset, s.size), (*tiling, off + shift, *size), "level {lvl}");
    }
}

#[test]
fn tfuplan_agrees_with_mesa_layout() {
    use texture71::Tiling as T;
    let (t, j) = (tex64(), plan64());
    assert_eq!(j.base_offset, u64::from(t.slices[0].offset));
    assert_eq!(j.levels.len(), 6);
    for l in &j.levels {
        let s = t.slices[l.level as usize];
        assert_eq!(l.offset, u64::from(s.offset), "level {} offset", l.level);
        assert_eq!(l.bytes, u64::from(s.size), "level {} size", l.level);
        let want = match s.tiling {
            T::LinearTile => tfuplan71::Tiling::LinearTile,
            T::UbLinear1Column => tfuplan71::Tiling::UbLinear1Column,
            T::UbLinear2Column => tfuplan71::Tiling::UbLinear2Column,
            T::UifNoXor => tfuplan71::Tiling::UifNoXor,
            T::UifXor => tfuplan71::Tiling::UifXor,
        };
        assert_eq!(l.tiling, want, "level {} tiling", l.level);
    }
}

#[test]
fn tfuplan_agrees_over_shapes() {
    // square, non-square and large chains, every cpp the TFU takes
    for &(w, h) in &[(64u32, 64u32), (256, 256), (128, 32), (100, 60), (1024, 1024), (2048, 520), (4096, 4096)] {
        for cpp in [1u32, 2, 4, 8, 16] {
            let n = w.max(h).ilog2();
            let t = texture71::layout(w, h, n + 1, cpp, 1).unwrap();
            let j = &tfuplan71::plan(&args(w, h, n, cpp)).unwrap()[0];
            assert_eq!(j.base_offset, u64::from(t.slices[0].offset), "{w}x{h} cpp {cpp}");
            for l in &j.levels {
                let s = t.slices[l.level as usize];
                assert_eq!(
                    (l.offset, l.bytes),
                    (u64::from(s.offset), u64::from(s.size)),
                    "{w}x{h} cpp {cpp} L{}",
                    l.level
                );
            }
        }
    }
}

#[test]
fn tex2d_reference_level0() {
    // gl_tex2d: 64x64 RGBA8, level 0 only: UIF no XOR, 8 UIF block rows
    let clif = read("tools/v3d/pi5/gl_tex2d.clif");
    assert!(clif.contains("Memory Format: */ 4 /* UIF (No XOR) */"));
    assert!(clif.contains("Height in UB or Stride: */ 8"));
    let t = texture71::layout(64, 64, 1, 4, 1).unwrap();
    let s = t.slices[0];
    assert_eq!((s.tiling, s.offset, s.size, s.stride), (texture71::Tiling::UifNoXor, 0, 16384, 256));
    assert_eq!(s.padded_height / 8, 8);
}
