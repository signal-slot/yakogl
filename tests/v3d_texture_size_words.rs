// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `texture::size_words`, the V3D size uniform words 44-47 (width, height,
//! depth, array size) of a texture bind at a first level, as a table (plan
//! #382 item sizewordsut, issue #1261). Hand-built `GlesTextureBind`s, no GL
//! context.
//!
//! Sources of every expected value:
//! - Mesa `get_texture_size`, v3d_uniforms.c:63-72 (tree): width, height and
//!   depth are `u_minify(size, first_level)` = `max(size >> first_level, 1)`;
//!   v3d_uniforms.c:73-79 (tree): the array size is the view's layer count,
//!   never minified. Depth is only asked for by a 3D sampler and the array
//!   size only by a 2D array sampler, so every other type gives 1.
//! - docs/notes/v3d-texture-size-words.md (b): an incomplete bind is
//!   1 / 1 / 1 / 1 (GL ES 2.0 3.7.10 black stand-in), `max(b.layers, 1)`
//!   is the count; (e): the words minify at the sampler view's first level.
//!
//! Every row agrees with Mesa, so no `_flip_when_fixed` test is needed.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_texture_size_words
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", feature = "host-test", not(feature = "report-es2")))]

use yakogl::backend::*;
use yakogl::v3d::texture::size_words;

fn bind(type_: GlesTexType, w: u32, h: u32, layers: u16, incomplete: u8) -> GlesTextureBind {
    GlesTextureBind { type_, width: w, height: h, layers, incomplete, ..Default::default() }
}

type Row = (&'static str, GlesTextureBind, u32, (u32, u32, u32, u32));

#[test]
fn size_words_table() {
    // (name, bind, first_level, expected (w, h, depth, array))
    // Minify rows: v3d_uniforms.c:63-72 (tree); array rows: :73-79 (tree).
    let t2 = |w, h| bind(GLES_TEXTYPE_2D, w, h, 1, 0);
    let t3 = |w, h, d| bind(GLES_TEXTYPE_3D, w, h, d, 0);
    let ta = |w, h, n| bind(GLES_TEXTYPE_2D_ARRAY, w, h, n, 0);
    let tc = |s| bind(GLES_TEXTYPE_CUBE, s, s, 6, 0);
    let inc = |t| bind(t, 16, 16, 4, 1);
    let rows: Vec<Row> = vec![
        // 2D: depth and array size 1 (note (b)).
        ("2d l0", t2(64, 32), 0, (64, 32, 1, 1)),
        ("2d l1", t2(64, 32), 1, (32, 16, 1, 1)),
        ("2d l5 height saturates", t2(64, 32), 5, (2, 1, 1, 1)),
        ("2d l6 past chain", t2(64, 32), 6, (1, 1, 1, 1)),
        ("2d l31", t2(64, 32), 31, (1, 1, 1, 1)),
        ("2d l32 shift saturates", t2(64, 32), 32, (1, 1, 1, 1)),
        ("2d l100", t2(64, 32), 100, (1, 1, 1, 1)),
        ("2d npot 37x21 l0", t2(37, 21), 0, (37, 21, 1, 1)),
        ("2d npot 37x21 l1", t2(37, 21), 1, (18, 10, 1, 1)),
        ("2d npot 37x21 l2", t2(37, 21), 2, (9, 5, 1, 1)),
        ("2d npot 37x21 l4", t2(37, 21), 4, (2, 1, 1, 1)),
        ("2d npot 37x21 l5", t2(37, 21), 5, (1, 1, 1, 1)),
        ("2d 1x1 l0", t2(1, 1), 0, (1, 1, 1, 1)),
        // 3D: depth is b.layers minified like width (:63-72).
        ("3d l0", t3(16, 8, 6), 0, (16, 8, 6, 1)),
        ("3d l1", t3(16, 8, 6), 1, (8, 4, 3, 1)),
        ("3d l2", t3(16, 8, 6), 2, (4, 2, 1, 1)),
        ("3d l3", t3(16, 8, 6), 3, (2, 1, 1, 1)),
        ("3d l9 past chain", t3(16, 8, 6), 9, (1, 1, 1, 1)),
        ("3d npot 9x5x7 l1", t3(9, 5, 7), 1, (4, 2, 3, 1)),
        ("3d npot 9x5x7 l2", t3(9, 5, 7), 2, (2, 1, 1, 1)),
        ("3d npot 9x5x7 l3", t3(9, 5, 7), 3, (1, 1, 1, 1)),
        ("3d deep 3x3x256 l4", t3(3, 3, 256), 4, (1, 1, 16, 1)),
        ("3d deep l32", t3(3, 3, 256), 32, (1, 1, 1, 1)),
        // 2D array: layers whole, never minified (:73-79).
        ("array l0", ta(16, 8, 5), 0, (16, 8, 1, 5)),
        ("array l1", ta(16, 8, 5), 1, (8, 4, 1, 5)),
        ("array l4 past height", ta(16, 8, 5), 4, (1, 1, 1, 5)),
        ("array l20 past chain", ta(16, 8, 5), 20, (1, 1, 1, 5)),
        ("array l32", ta(16, 8, 5), 32, (1, 1, 1, 5)),
        ("array 256 layers l3", ta(32, 32, 256), 3, (4, 4, 1, 256)),
        ("array 1 layer l1", ta(8, 8, 1), 1, (4, 4, 1, 1)),
        // Cube: depth and array size 1 whatever b.layers (6) is.
        ("cube l0", tc(32), 0, (32, 32, 1, 1)),
        ("cube l2", tc(32), 2, (8, 8, 1, 1)),
        ("cube l5", tc(32), 5, (1, 1, 1, 1)),
        ("cube l7 past chain", tc(32), 7, (1, 1, 1, 1)),
        ("cube npot l1", tc(5), 1, (2, 2, 1, 1)),
        // layers == 0 counts as 1 (`max(layers, 1)`, note (b)).
        ("3d layers 0", bind(GLES_TEXTYPE_3D, 8, 8, 0, 0), 0, (8, 8, 1, 1)),
        ("3d layers 0 l1", bind(GLES_TEXTYPE_3D, 8, 8, 0, 0), 1, (4, 4, 1, 1)),
        ("array layers 0", bind(GLES_TEXTYPE_2D_ARRAY, 8, 8, 0, 0), 0, (8, 8, 1, 1)),
        ("array layers 0 l2", bind(GLES_TEXTYPE_2D_ARRAY, 8, 8, 0, 0), 2, (2, 2, 1, 1)),
        ("2d layers 0", bind(GLES_TEXTYPE_2D, 8, 8, 0, 0), 0, (8, 8, 1, 1)),
        // Incomplete: the 1x1x1x1 black stand-in at any level (note (b)).
        ("incomplete 2d l0", inc(GLES_TEXTYPE_2D), 0, (1, 1, 1, 1)),
        ("incomplete 2d l2", inc(GLES_TEXTYPE_2D), 2, (1, 1, 1, 1)),
        ("incomplete 3d l0", inc(GLES_TEXTYPE_3D), 0, (1, 1, 1, 1)),
        ("incomplete 3d l1", inc(GLES_TEXTYPE_3D), 1, (1, 1, 1, 1)),
        ("incomplete array l0", inc(GLES_TEXTYPE_2D_ARRAY), 0, (1, 1, 1, 1)),
        ("incomplete array l3", inc(GLES_TEXTYPE_2D_ARRAY), 3, (1, 1, 1, 1)),
        ("incomplete cube l0", inc(GLES_TEXTYPE_CUBE), 0, (1, 1, 1, 1)),
        ("incomplete cube l40", inc(GLES_TEXTYPE_CUBE), 40, (1, 1, 1, 1)),
    ];
    let mut bad = Vec::new();
    for (name, b, level, want) in &rows {
        let got = size_words(b, *level);
        if got != *want {
            bad.push(format!("{name}: first level {level}: got {got:?}, want {want:?}"));
        }
    }
    assert!(bad.is_empty(), "{} row(s) differ:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn the_table_is_not_trivially_one() {
    // Guard against a stub returning 1s: level 0 of a 3D texture is its size.
    let b = bind(GLES_TEXTYPE_3D, 16, 8, 6, 0);
    assert_eq!(size_words(&b, 0), (16, 8, 6, 1));
}
