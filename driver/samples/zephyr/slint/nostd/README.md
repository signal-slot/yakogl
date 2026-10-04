# no_std Slint / FemtoVG / glow forks for the GPU samples

`samples/zephyr/slint` (the one Slint sample), `cluster`, `v3d_gl_femtovg` and
`samples/zephyr/gl_femtovg` link Rust crates that
do not exist upstream in a `no_std` form. They are built from three patched
trees that live *outside* this repository (they are large multi-crate
checkouts); this directory carries everything needed to recreate them.

| Tree | Base | Patch | Points the CMake at |
|------|------|-------|---------------------|
| Slint (core, common, compiler, FemtoVG renderer, `slint` facade, `slint-build`) | `slint-ui/slint` commit `1007c032ca1573e60dce2113e802e0ccb91c2641` (upstream, 2026-08-17) | `slint-nostd.patch` | `SLINT_NOSTD_DIR` |
| femtovg 0.26.0 | crates.io `femtovg-0.26.0` | `femtovg-0.26.0-nostd.patch` | `NOSTD_PATCH_DIR/femtovg` |
| glow 0.18.0 | crates.io `glow-0.18.0` | `glow-0.18.0-nostd.patch` | `NOSTD_PATCH_DIR/glow` |

What the patches do:

- **Slint core / common**: `#![no_std]` + `alloc` (hashbrown instead of
  `std::collections`, `libm` maths), the std-only text stack (parley,
  fontique, skrifa), image decoders and SVG gated off.
- **Slint FemtoVG renderer**: `no_std`; text through compile-time bitmap
  fonts (`internal/renderers/femtovg/bitmapfont.rs`: the `EmbedForSoftwareRenderer`
  glyphs laid out with `i_slint_core::textlayout` and composed into one texture
  per `Text` item); conic gradients start at 12 o'clock as the Slint
  specification says (femtovg's start at 3 o'clock). A Text's texture holds
  only the part of its box with ink (drawn at its offset), and
  `FemtoVGRenderer::set_skip_window_clear` lets an application whose
  BeforeRendering notifier covers the window skip the clear (gl_slint's
  `slint_set_underlay_covers`).
- **Slint compiler** (`embed_glyphs.rs`): static (non-variable) fonts are
  embedded once per requested weight, so Noto Sans Light/Regular/SemiBold all
  land in the image (upstream only did that for variable fonts).
- **femtovg**: `no_std` + `alloc`; the swash text stack and std-only APIs
  removed; NO_DISCARD and NO_SCISSOR fill programs (tools/shaders/femtovg has
  their texts); gradient tables pooled, a pooled one refilled two flushes
  after release; a `TRACE`-free build (the GL trace hook used for debugging is not
  part of the patch).
- **glow**: mechanical `no_std` port (`core::` instead of `std::`).

## Recreating the workspaces

```sh
# 1. Slint: upstream at the commit above, then the patch
git clone https://github.com/slint-ui/slint.git $WORK/slint-nostd/slint
(cd $WORK/slint-nostd/slint && git checkout 1007c032ca1573e60dce2113e802e0ccb91c2641 &&
 patch -p1 < <driver>/samples/zephyr/slint/nostd/slint-nostd.patch)

# 2. femtovg / glow: the crates.io sources are in the cargo registry
mkdir -p $WORK/nostd && cd $WORK/nostd
cp -r ~/.cargo/registry/src/*/femtovg-0.26.0 femtovg && (cd femtovg && patch -p1 < .../femtovg-0.26.0-nostd.patch)
cp -r ~/.cargo/registry/src/*/glow-0.18.0 glow && (cd glow && patch -p1 < .../glow-0.18.0-nostd.patch)
cp .../nostd-workspace.Cargo.toml Cargo.toml       # workspace: fvg_demo (+ probe)
cp -r .../fvg_demo fvg_demo                        # the raw FemtoVG demo crate of gl_femtovg

# 3. build the samples
west build -b verdin_am62p/am62p54/a53 -d build/slint samples/zephyr/slint -- \
    -DSLINT_NOSTD_DIR=$WORK/slint-nostd/slint -DNOSTD_PATCH_DIR=$WORK/nostd   # or -b rpi_4b, verdin_imx8mp/mimx8ml8/a53
west build -b verdin_am62p/am62p54/a53 -d build/gl_femtovg samples/zephyr/gl_femtovg -- \
    -DFVG_NOSTD_DIR=$WORK/nostd
```

Toolchain: nightly `cargo` (`-Z build-std=core,alloc`), target
`aarch64-unknown-none`, and Noto Sans installed on the host (the Slint
compiler pre-renders the cluster's fonts at build time).

**Where the Slint base came from.** The tree the boards' images were built
from (a development host's `slint-nostd`) is a partial copy of an upstream checkout plus
the no_std work, and it was not at `be50855` as this file used to say: the
earlier patch no longer reproduced it (67 files differed). On 2026-09-23 every
file of that tree was hashed and compared against each upstream commit from
`be50855` on; `1007c03` matches the most files (868 of 966), and the patch is
the diff from it to the tree, file for file. The tree also carries files from
slightly later upstream compiler commits, which are therefore in the patch
too. Applied to a fresh `1007c03` checkout it reproduces all 960 text files of
the tree byte for byte, and gc_slint builds from the result. Six binary assets
(`slint-ui-app/ui-cluster/assets/*`, one compiler test image) are not in the
patch; no sample builds them.

The femtovg and glow patches in the table above were checked the same way on 2026-09-23
and still reproduce the trees exactly.

The patches were generated with `diff -ruN` against the bases above
(excluding `target/`, `Cargo.lock` and `LICENSES/`); keep them in sync when
the trees change. A change to files the patch already names goes through
`splice_patch.py <patch> <base> <tree> <file>...`: it regenerates only those
files' sections and keeps every other section and the order byte for byte (a
full `diff -ruN` reorders slint-nostd.patch and rewrites the femtovg patch's
timestamps). Then apply `<patch>.new` to a fresh copy of the base, compare it
with the tree (`diff -rq -x .git -x target -x .target -x Cargo.lock`, 0 files
may differ), and move it over the patch.
