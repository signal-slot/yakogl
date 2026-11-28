// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Word and thread counts of every glmark2-es2 scene program through the V3D
//! 4.2 (Raspberry Pi 4) online compiler, pinned as a table.
//!
//! The glmark2 texts are GPL-3.0 and never committed. They come from the
//! corpus `tools/hostharness/glmark2/run.sh "$GLMARK2_SRC"` writes to
//! `build/glmark2-host/shaders/` (`GLMARK2_CORPUS` names another directory
//! holding `index.txt`), which holds the text glmark2 hands to
//! `glShaderSource` per scene (entry). Every vertex/fragment pair of one
//! entry that links is one program (the index does not say which shaders
//! meet, so the cross product is tried; a pair whose fragment shader reads a
//! varying the vertex shader does not write is not a program glmark2 builds).
//! Terrain's noise and draw programs are in the table too
//! (`tests/v3d_terrain_sched_words.rs` pins them on their own).
//!
//! With no corpus (and no `GLMARK2_CORPUS`) the test prints a SKIP line and
//! returns, as `tests/glmark2_corpus.rs` does. With `--nocapture` it prints
//! the table it measured, one row per stage of every program.
//!
//! Updating: a change to the V3D scheduler or the register allocator moves
//! these counts on purpose. Read the "moved" lines of the failure (program,
//! stage, old and new words and threads), then replace the rows of `PINS`
//! with the printed table (`--nocapture`: lines starting `PIN `) in the same
//! commit as the change, as the word pins of the other V3D tests are.
//! A program that appears or disappears fails with its name too.
//!
//! ```text
//! GLMARK2_SRC=... tools/hostharness/glmark2/run.sh "$GLMARK2_SRC"   # once
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online --test v3d_glmark2_words -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::{lower, sema};

const TARGET: Target = Target { vpm_size: 65536 };

/// (scene, program `<vertex file>+<fragment file>`, stage, words, threads).
/// Stages: `fs`, `vs`, `cs` (the coordinate shader).
const PINS: &[(&str, &str, &str, usize, u32)] = &[
    (
        "buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false",
        "buffer-723443fdbaf22e8b.vert.glsl+buffer-9fe66cb469a36a66.frag.glsl",
        "fs",
        23,
        4,
    ),
    (
        "buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false",
        "buffer-723443fdbaf22e8b.vert.glsl+buffer-9fe66cb469a36a66.frag.glsl",
        "vs",
        147,
        4,
    ),
    (
        "buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false",
        "buffer-723443fdbaf22e8b.vert.glsl+buffer-9fe66cb469a36a66.frag.glsl",
        "cs",
        36,
        4,
    ),
    (
        "buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true",
        "buffer-723443fdbaf22e8b.vert.glsl+buffer-9fe66cb469a36a66.frag.glsl",
        "fs",
        23,
        4,
    ),
    (
        "buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true",
        "buffer-723443fdbaf22e8b.vert.glsl+buffer-9fe66cb469a36a66.frag.glsl",
        "vs",
        147,
        4,
    ),
    (
        "buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true",
        "buffer-723443fdbaf22e8b.vert.glsl+buffer-9fe66cb469a36a66.frag.glsl",
        "cs",
        36,
        4,
    ),
    (
        "buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false",
        "buffer-723443fdbaf22e8b.vert.glsl+buffer-9fe66cb469a36a66.frag.glsl",
        "fs",
        23,
        4,
    ),
    (
        "buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false",
        "buffer-723443fdbaf22e8b.vert.glsl+buffer-9fe66cb469a36a66.frag.glsl",
        "vs",
        147,
        4,
    ),
    (
        "buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false",
        "buffer-723443fdbaf22e8b.vert.glsl+buffer-9fe66cb469a36a66.frag.glsl",
        "cs",
        36,
        4,
    ),
    ("build:use-vbo=false", "build-ace204f6171a58b1.vert.glsl+build-474800a67afed087.frag.glsl", "fs", 9, 4),
    ("build:use-vbo=false", "build-ace204f6171a58b1.vert.glsl+build-474800a67afed087.frag.glsl", "vs", 68, 4),
    ("build:use-vbo=false", "build-ace204f6171a58b1.vert.glsl+build-474800a67afed087.frag.glsl", "cs", 36, 4),
    ("build:use-vbo=true", "build-ace204f6171a58b1.vert.glsl+build-474800a67afed087.frag.glsl", "fs", 9, 4),
    ("build:use-vbo=true", "build-ace204f6171a58b1.vert.glsl+build-474800a67afed087.frag.glsl", "vs", 68, 4),
    ("build:use-vbo=true", "build-ace204f6171a58b1.vert.glsl+build-474800a67afed087.frag.glsl", "cs", 36, 4),
    ("bump:bump-render=height", "bump-270aae3cfcae71a2.vert.glsl+bump-77c536a2dad6a903.frag.glsl", "fs", 61, 4),
    ("bump:bump-render=height", "bump-270aae3cfcae71a2.vert.glsl+bump-77c536a2dad6a903.frag.glsl", "vs", 123, 4),
    ("bump:bump-render=height", "bump-270aae3cfcae71a2.vert.glsl+bump-77c536a2dad6a903.frag.glsl", "cs", 36, 4),
    ("bump:bump-render=high-poly", "shading-78e3ba59e507a5cc.vert.glsl+bump-d4424181c7796637.frag.glsl", "fs", 33, 4),
    ("bump:bump-render=high-poly", "shading-78e3ba59e507a5cc.vert.glsl+bump-d4424181c7796637.frag.glsl", "vs", 61, 4),
    ("bump:bump-render=high-poly", "shading-78e3ba59e507a5cc.vert.glsl+bump-d4424181c7796637.frag.glsl", "cs", 36, 4),
    ("bump:bump-render=normals", "bump-51d0a6207c34d50c.vert.glsl+bump-487d34975c3c8a1e.frag.glsl", "fs", 54, 4),
    ("bump:bump-render=normals", "bump-51d0a6207c34d50c.vert.glsl+bump-487d34975c3c8a1e.frag.glsl", "vs", 41, 4),
    ("bump:bump-render=normals", "bump-51d0a6207c34d50c.vert.glsl+bump-487d34975c3c8a1e.frag.glsl", "cs", 36, 4),
    (
        "conditionals:vertex-steps=0:fragment-steps=0",
        "conditionals-310736a638617746.vert.glsl+conditionals-6c61e8344a883119.frag.glsl",
        "fs",
        10,
        4,
    ),
    (
        "conditionals:vertex-steps=0:fragment-steps=0",
        "conditionals-310736a638617746.vert.glsl+conditionals-6c61e8344a883119.frag.glsl",
        "vs",
        39,
        4,
    ),
    (
        "conditionals:vertex-steps=0:fragment-steps=0",
        "conditionals-310736a638617746.vert.glsl+conditionals-6c61e8344a883119.frag.glsl",
        "cs",
        38,
        4,
    ),
    (
        "conditionals:vertex-steps=0:fragment-steps=5",
        "conditionals-310736a638617746.vert.glsl+conditionals-a5b5e74601fb18a9.frag.glsl",
        "fs",
        40,
        4,
    ),
    (
        "conditionals:vertex-steps=0:fragment-steps=5",
        "conditionals-310736a638617746.vert.glsl+conditionals-a5b5e74601fb18a9.frag.glsl",
        "vs",
        39,
        4,
    ),
    (
        "conditionals:vertex-steps=0:fragment-steps=5",
        "conditionals-310736a638617746.vert.glsl+conditionals-a5b5e74601fb18a9.frag.glsl",
        "cs",
        38,
        4,
    ),
    (
        "conditionals:vertex-steps=5:fragment-steps=0",
        "conditionals-a21c916ffe4265bc.vert.glsl+conditionals-6c61e8344a883119.frag.glsl",
        "fs",
        10,
        4,
    ),
    (
        "conditionals:vertex-steps=5:fragment-steps=0",
        "conditionals-a21c916ffe4265bc.vert.glsl+conditionals-6c61e8344a883119.frag.glsl",
        "vs",
        67,
        4,
    ),
    (
        "conditionals:vertex-steps=5:fragment-steps=0",
        "conditionals-a21c916ffe4265bc.vert.glsl+conditionals-6c61e8344a883119.frag.glsl",
        "cs",
        66,
        4,
    ),
    (
        "desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true",
        "desktop-792b1bd04a03eaaa.vert.glsl+desktop-68afdb5a2dcf8400.frag.glsl",
        "fs",
        86,
        4,
    ),
    (
        "desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true",
        "desktop-792b1bd04a03eaaa.vert.glsl+desktop-68afdb5a2dcf8400.frag.glsl",
        "vs",
        22,
        4,
    ),
    (
        "desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true",
        "desktop-792b1bd04a03eaaa.vert.glsl+desktop-68afdb5a2dcf8400.frag.glsl",
        "cs",
        20,
        4,
    ),
    (
        "desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true",
        "desktop-792b1bd04a03eaaa.vert.glsl+desktop-7f177aebfac60da2.frag.glsl",
        "fs",
        85,
        4,
    ),
    (
        "desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true",
        "desktop-792b1bd04a03eaaa.vert.glsl+desktop-7f177aebfac60da2.frag.glsl",
        "vs",
        22,
        4,
    ),
    (
        "desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true",
        "desktop-792b1bd04a03eaaa.vert.glsl+desktop-7f177aebfac60da2.frag.glsl",
        "cs",
        20,
        4,
    ),
    (
        "desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true",
        "desktop-792b1bd04a03eaaa.vert.glsl+desktop-9ce90a0e44b5ae05.frag.glsl",
        "fs",
        11,
        4,
    ),
    (
        "desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true",
        "desktop-792b1bd04a03eaaa.vert.glsl+desktop-9ce90a0e44b5ae05.frag.glsl",
        "vs",
        22,
        4,
    ),
    (
        "desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true",
        "desktop-792b1bd04a03eaaa.vert.glsl+desktop-9ce90a0e44b5ae05.frag.glsl",
        "cs",
        20,
        4,
    ),
    (
        "desktop:windows=4:effect=shadow",
        "desktop-792b1bd04a03eaaa.vert.glsl+desktop-9ce90a0e44b5ae05.frag.glsl",
        "fs",
        11,
        4,
    ),
    (
        "desktop:windows=4:effect=shadow",
        "desktop-792b1bd04a03eaaa.vert.glsl+desktop-9ce90a0e44b5ae05.frag.glsl",
        "vs",
        22,
        4,
    ),
    (
        "desktop:windows=4:effect=shadow",
        "desktop-792b1bd04a03eaaa.vert.glsl+desktop-9ce90a0e44b5ae05.frag.glsl",
        "cs",
        20,
        4,
    ),
    (
        "effect2d:kernel=0,1,0;1,-4,1;0,1,0;",
        "effect2d-46239418fca1dc03.vert.glsl+effect2d-3c3d57ba2c0ece72.frag.glsl",
        "fs",
        75,
        4,
    ),
    (
        "effect2d:kernel=0,1,0;1,-4,1;0,1,0;",
        "effect2d-46239418fca1dc03.vert.glsl+effect2d-3c3d57ba2c0ece72.frag.glsl",
        "vs",
        23,
        4,
    ),
    (
        "effect2d:kernel=0,1,0;1,-4,1;0,1,0;",
        "effect2d-46239418fca1dc03.vert.glsl+effect2d-3c3d57ba2c0ece72.frag.glsl",
        "cs",
        20,
        4,
    ),
    (
        "effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;",
        "effect2d-46239418fca1dc03.vert.glsl+effect2d-ba8e6925e955f7f2.frag.glsl",
        "fs",
        106,
        4,
    ),
    (
        "effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;",
        "effect2d-46239418fca1dc03.vert.glsl+effect2d-ba8e6925e955f7f2.frag.glsl",
        "vs",
        23,
        4,
    ),
    (
        "effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;",
        "effect2d-46239418fca1dc03.vert.glsl+effect2d-ba8e6925e955f7f2.frag.glsl",
        "cs",
        20,
        4,
    ),
    (
        "function:fragment-steps=5:fragment-complexity=low",
        "function-016263068a7fa619.vert.glsl+function-c8ce6c64e0b9bbb0.frag.glsl",
        "fs",
        25,
        4,
    ),
    (
        "function:fragment-steps=5:fragment-complexity=low",
        "function-016263068a7fa619.vert.glsl+function-c8ce6c64e0b9bbb0.frag.glsl",
        "vs",
        41,
        4,
    ),
    (
        "function:fragment-steps=5:fragment-complexity=low",
        "function-016263068a7fa619.vert.glsl+function-c8ce6c64e0b9bbb0.frag.glsl",
        "cs",
        41,
        4,
    ),
    (
        "function:fragment-steps=5:fragment-complexity=medium",
        "function-016263068a7fa619.vert.glsl+function-849c78149ac0211c.frag.glsl",
        "fs",
        60,
        4,
    ),
    (
        "function:fragment-steps=5:fragment-complexity=medium",
        "function-016263068a7fa619.vert.glsl+function-849c78149ac0211c.frag.glsl",
        "vs",
        41,
        4,
    ),
    (
        "function:fragment-steps=5:fragment-complexity=medium",
        "function-016263068a7fa619.vert.glsl+function-849c78149ac0211c.frag.glsl",
        "cs",
        41,
        4,
    ),
    ("ideas:speed=duration", "ideas-038e5d26ce5671ff.vert.glsl+ideas-23c87c182efb90f6.frag.glsl", "fs", 9, 4),
    ("ideas:speed=duration", "ideas-038e5d26ce5671ff.vert.glsl+ideas-23c87c182efb90f6.frag.glsl", "vs", 131, 4),
    ("ideas:speed=duration", "ideas-038e5d26ce5671ff.vert.glsl+ideas-23c87c182efb90f6.frag.glsl", "cs", 127, 4),
    ("ideas:speed=duration", "ideas-038e5d26ce5671ff.vert.glsl+ideas-df64b7201645108a.frag.glsl", "fs", 15, 4),
    ("ideas:speed=duration", "ideas-038e5d26ce5671ff.vert.glsl+ideas-df64b7201645108a.frag.glsl", "vs", 127, 4),
    ("ideas:speed=duration", "ideas-038e5d26ce5671ff.vert.glsl+ideas-df64b7201645108a.frag.glsl", "cs", 127, 4),
    ("ideas:speed=duration", "ideas-038e5d26ce5671ff.vert.glsl+ideas-e6a9ec772d9b5d00.frag.glsl", "fs", 9, 4),
    ("ideas:speed=duration", "ideas-038e5d26ce5671ff.vert.glsl+ideas-e6a9ec772d9b5d00.frag.glsl", "vs", 127, 4),
    ("ideas:speed=duration", "ideas-038e5d26ce5671ff.vert.glsl+ideas-e6a9ec772d9b5d00.frag.glsl", "cs", 127, 4),
    ("ideas:speed=duration", "ideas-4f27e86c3643a88a.vert.glsl+ideas-df64b7201645108a.frag.glsl", "fs", 15, 4),
    ("ideas:speed=duration", "ideas-4f27e86c3643a88a.vert.glsl+ideas-df64b7201645108a.frag.glsl", "vs", 127, 4),
    ("ideas:speed=duration", "ideas-4f27e86c3643a88a.vert.glsl+ideas-df64b7201645108a.frag.glsl", "cs", 127, 4),
    ("ideas:speed=duration", "ideas-4f27e86c3643a88a.vert.glsl+ideas-e6a9ec772d9b5d00.frag.glsl", "fs", 9, 4),
    ("ideas:speed=duration", "ideas-4f27e86c3643a88a.vert.glsl+ideas-e6a9ec772d9b5d00.frag.glsl", "vs", 127, 4),
    ("ideas:speed=duration", "ideas-4f27e86c3643a88a.vert.glsl+ideas-e6a9ec772d9b5d00.frag.glsl", "cs", 127, 4),
    ("ideas:speed=duration", "ideas-51b217c5b7686924.vert.glsl+ideas-1acaaccc424c9602.frag.glsl", "fs", 190, 4),
    ("ideas:speed=duration", "ideas-51b217c5b7686924.vert.glsl+ideas-1acaaccc424c9602.frag.glsl", "vs", 159, 2),
    ("ideas:speed=duration", "ideas-51b217c5b7686924.vert.glsl+ideas-1acaaccc424c9602.frag.glsl", "cs", 127, 4),
    ("ideas:speed=duration", "ideas-51b217c5b7686924.vert.glsl+ideas-c5652010d501f7e4.frag.glsl", "fs", 79, 4),
    ("ideas:speed=duration", "ideas-51b217c5b7686924.vert.glsl+ideas-c5652010d501f7e4.frag.glsl", "vs", 159, 2),
    ("ideas:speed=duration", "ideas-51b217c5b7686924.vert.glsl+ideas-c5652010d501f7e4.frag.glsl", "cs", 127, 4),
    ("ideas:speed=duration", "ideas-51b217c5b7686924.vert.glsl+ideas-df64b7201645108a.frag.glsl", "fs", 15, 4),
    ("ideas:speed=duration", "ideas-51b217c5b7686924.vert.glsl+ideas-df64b7201645108a.frag.glsl", "vs", 127, 4),
    ("ideas:speed=duration", "ideas-51b217c5b7686924.vert.glsl+ideas-df64b7201645108a.frag.glsl", "cs", 127, 4),
    ("ideas:speed=duration", "ideas-51b217c5b7686924.vert.glsl+ideas-e6a9ec772d9b5d00.frag.glsl", "fs", 9, 4),
    ("ideas:speed=duration", "ideas-51b217c5b7686924.vert.glsl+ideas-e6a9ec772d9b5d00.frag.glsl", "vs", 127, 4),
    ("ideas:speed=duration", "ideas-51b217c5b7686924.vert.glsl+ideas-e6a9ec772d9b5d00.frag.glsl", "cs", 127, 4),
    ("ideas:speed=duration", "ideas-a09e06caff031f78.vert.glsl+ideas-23c87c182efb90f6.frag.glsl", "fs", 9, 4),
    ("ideas:speed=duration", "ideas-a09e06caff031f78.vert.glsl+ideas-23c87c182efb90f6.frag.glsl", "vs", 134, 4),
    ("ideas:speed=duration", "ideas-a09e06caff031f78.vert.glsl+ideas-23c87c182efb90f6.frag.glsl", "cs", 126, 4),
    ("ideas:speed=duration", "ideas-a09e06caff031f78.vert.glsl+ideas-df64b7201645108a.frag.glsl", "fs", 15, 4),
    ("ideas:speed=duration", "ideas-a09e06caff031f78.vert.glsl+ideas-df64b7201645108a.frag.glsl", "vs", 127, 4),
    ("ideas:speed=duration", "ideas-a09e06caff031f78.vert.glsl+ideas-df64b7201645108a.frag.glsl", "cs", 126, 4),
    ("ideas:speed=duration", "ideas-a09e06caff031f78.vert.glsl+ideas-e6a9ec772d9b5d00.frag.glsl", "fs", 9, 4),
    ("ideas:speed=duration", "ideas-a09e06caff031f78.vert.glsl+ideas-e6a9ec772d9b5d00.frag.glsl", "vs", 127, 4),
    ("ideas:speed=duration", "ideas-a09e06caff031f78.vert.glsl+ideas-e6a9ec772d9b5d00.frag.glsl", "cs", 126, 4),
    ("ideas:speed=duration", "ideas-bf4bdbb017a1985a.vert.glsl+ideas-23c87c182efb90f6.frag.glsl", "fs", 9, 4),
    ("ideas:speed=duration", "ideas-bf4bdbb017a1985a.vert.glsl+ideas-23c87c182efb90f6.frag.glsl", "vs", 157, 4),
    ("ideas:speed=duration", "ideas-bf4bdbb017a1985a.vert.glsl+ideas-23c87c182efb90f6.frag.glsl", "cs", 127, 4),
    ("ideas:speed=duration", "ideas-bf4bdbb017a1985a.vert.glsl+ideas-df64b7201645108a.frag.glsl", "fs", 15, 4),
    ("ideas:speed=duration", "ideas-bf4bdbb017a1985a.vert.glsl+ideas-df64b7201645108a.frag.glsl", "vs", 127, 4),
    ("ideas:speed=duration", "ideas-bf4bdbb017a1985a.vert.glsl+ideas-df64b7201645108a.frag.glsl", "cs", 127, 4),
    ("ideas:speed=duration", "ideas-bf4bdbb017a1985a.vert.glsl+ideas-e6a9ec772d9b5d00.frag.glsl", "fs", 9, 4),
    ("ideas:speed=duration", "ideas-bf4bdbb017a1985a.vert.glsl+ideas-e6a9ec772d9b5d00.frag.glsl", "vs", 127, 4),
    ("ideas:speed=duration", "ideas-bf4bdbb017a1985a.vert.glsl+ideas-e6a9ec772d9b5d00.frag.glsl", "cs", 127, 4),
    ("ideas:speed=duration", "ideas-c23687991226ad5a.vert.glsl+ideas-23c87c182efb90f6.frag.glsl", "fs", 9, 4),
    ("ideas:speed=duration", "ideas-c23687991226ad5a.vert.glsl+ideas-23c87c182efb90f6.frag.glsl", "vs", 131, 4),
    ("ideas:speed=duration", "ideas-c23687991226ad5a.vert.glsl+ideas-23c87c182efb90f6.frag.glsl", "cs", 127, 4),
    ("ideas:speed=duration", "ideas-c23687991226ad5a.vert.glsl+ideas-df64b7201645108a.frag.glsl", "fs", 15, 4),
    ("ideas:speed=duration", "ideas-c23687991226ad5a.vert.glsl+ideas-df64b7201645108a.frag.glsl", "vs", 127, 4),
    ("ideas:speed=duration", "ideas-c23687991226ad5a.vert.glsl+ideas-df64b7201645108a.frag.glsl", "cs", 127, 4),
    ("ideas:speed=duration", "ideas-c23687991226ad5a.vert.glsl+ideas-e6a9ec772d9b5d00.frag.glsl", "fs", 9, 4),
    ("ideas:speed=duration", "ideas-c23687991226ad5a.vert.glsl+ideas-e6a9ec772d9b5d00.frag.glsl", "vs", 127, 4),
    ("ideas:speed=duration", "ideas-c23687991226ad5a.vert.glsl+ideas-e6a9ec772d9b5d00.frag.glsl", "cs", 127, 4),
    ("ideas:speed=duration", "ideas-c265c68a7991e5ae.vert.glsl+ideas-23c87c182efb90f6.frag.glsl", "fs", 9, 4),
    ("ideas:speed=duration", "ideas-c265c68a7991e5ae.vert.glsl+ideas-23c87c182efb90f6.frag.glsl", "vs", 158, 4),
    ("ideas:speed=duration", "ideas-c265c68a7991e5ae.vert.glsl+ideas-23c87c182efb90f6.frag.glsl", "cs", 127, 4),
    ("ideas:speed=duration", "ideas-c265c68a7991e5ae.vert.glsl+ideas-df64b7201645108a.frag.glsl", "fs", 15, 4),
    ("ideas:speed=duration", "ideas-c265c68a7991e5ae.vert.glsl+ideas-df64b7201645108a.frag.glsl", "vs", 127, 4),
    ("ideas:speed=duration", "ideas-c265c68a7991e5ae.vert.glsl+ideas-df64b7201645108a.frag.glsl", "cs", 127, 4),
    ("ideas:speed=duration", "ideas-c265c68a7991e5ae.vert.glsl+ideas-e6a9ec772d9b5d00.frag.glsl", "fs", 9, 4),
    ("ideas:speed=duration", "ideas-c265c68a7991e5ae.vert.glsl+ideas-e6a9ec772d9b5d00.frag.glsl", "vs", 127, 4),
    ("ideas:speed=duration", "ideas-c265c68a7991e5ae.vert.glsl+ideas-e6a9ec772d9b5d00.frag.glsl", "cs", 127, 4),
    ("jellyfish", "jellyfish-08a26355a7bc5cc3.vert.glsl+jellyfish-22fca12963ebb4df.frag.glsl", "fs", 37, 4),
    ("jellyfish", "jellyfish-08a26355a7bc5cc3.vert.glsl+jellyfish-22fca12963ebb4df.frag.glsl", "vs", 179, 4),
    ("jellyfish", "jellyfish-08a26355a7bc5cc3.vert.glsl+jellyfish-22fca12963ebb4df.frag.glsl", "cs", 73, 4),
    ("jellyfish", "jellyfish-14accf178aebd0fd.vert.glsl+jellyfish-8ee578801bfa8bdc.frag.glsl", "fs", 14, 4),
    ("jellyfish", "jellyfish-14accf178aebd0fd.vert.glsl+jellyfish-8ee578801bfa8bdc.frag.glsl", "vs", 22, 4),
    ("jellyfish", "jellyfish-14accf178aebd0fd.vert.glsl+jellyfish-8ee578801bfa8bdc.frag.glsl", "cs", 19, 4),
    (
        "loop:vertex-steps=5:fragment-steps=5:fragment-loop=false",
        "loop-3cb095bbd6bfdb49.vert.glsl+loop-e016aee79842b644.frag.glsl",
        "fs",
        25,
        4,
    ),
    (
        "loop:vertex-steps=5:fragment-steps=5:fragment-loop=false",
        "loop-3cb095bbd6bfdb49.vert.glsl+loop-e016aee79842b644.frag.glsl",
        "vs",
        61,
        4,
    ),
    (
        "loop:vertex-steps=5:fragment-steps=5:fragment-loop=false",
        "loop-3cb095bbd6bfdb49.vert.glsl+loop-e016aee79842b644.frag.glsl",
        "cs",
        60,
        4,
    ),
    (
        "loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false",
        "loop-3cb095bbd6bfdb49.vert.glsl+loop-c7eae5f6b11b98b4.frag.glsl",
        "fs",
        25,
        4,
    ),
    (
        "loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false",
        "loop-3cb095bbd6bfdb49.vert.glsl+loop-c7eae5f6b11b98b4.frag.glsl",
        "vs",
        61,
        4,
    ),
    (
        "loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false",
        "loop-3cb095bbd6bfdb49.vert.glsl+loop-c7eae5f6b11b98b4.frag.glsl",
        "cs",
        60,
        4,
    ),
    (
        "loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true",
        "loop-3cb095bbd6bfdb49.vert.glsl+loop-3678074e44f2c92c.frag.glsl",
        "fs",
        27,
        4,
    ),
    (
        "loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true",
        "loop-3cb095bbd6bfdb49.vert.glsl+loop-3678074e44f2c92c.frag.glsl",
        "vs",
        61,
        4,
    ),
    (
        "loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true",
        "loop-3cb095bbd6bfdb49.vert.glsl+loop-3678074e44f2c92c.frag.glsl",
        "cs",
        60,
        4,
    ),
    (
        "pulsar:quads=5:texture=false:light=false",
        "pulsar-9d75bb1847b5cc25.vert.glsl+build-474800a67afed087.frag.glsl",
        "fs",
        9,
        4,
    ),
    (
        "pulsar:quads=5:texture=false:light=false",
        "pulsar-9d75bb1847b5cc25.vert.glsl+build-474800a67afed087.frag.glsl",
        "vs",
        45,
        4,
    ),
    (
        "pulsar:quads=5:texture=false:light=false",
        "pulsar-9d75bb1847b5cc25.vert.glsl+build-474800a67afed087.frag.glsl",
        "cs",
        36,
        4,
    ),
    ("refract", "refract-7215f4f5052a383c.vert.glsl+refract-3fae89c9ab969ea3.frag.glsl", "fs", 125, 4),
    ("refract", "refract-7215f4f5052a383c.vert.glsl+refract-3fae89c9ab969ea3.frag.glsl", "vs", 96, 4),
    ("refract", "refract-7215f4f5052a383c.vert.glsl+refract-3fae89c9ab969ea3.frag.glsl", "cs", 36, 4),
    ("refract", "shadow-37f46ae8961221f2.vert.glsl+shadow-928e172a68b7f175.frag.glsl", "fs", 8, 4),
    ("refract", "shadow-37f46ae8961221f2.vert.glsl+shadow-928e172a68b7f175.frag.glsl", "vs", 43, 4),
    ("refract", "shadow-37f46ae8961221f2.vert.glsl+shadow-928e172a68b7f175.frag.glsl", "cs", 36, 4),
    (
        "shading:shading=blinn-phong-inf",
        "shading-78e3ba59e507a5cc.vert.glsl+shading-994f0197ada4c9fa.frag.glsl",
        "fs",
        34,
        4,
    ),
    (
        "shading:shading=blinn-phong-inf",
        "shading-78e3ba59e507a5cc.vert.glsl+shading-994f0197ada4c9fa.frag.glsl",
        "vs",
        61,
        4,
    ),
    (
        "shading:shading=blinn-phong-inf",
        "shading-78e3ba59e507a5cc.vert.glsl+shading-994f0197ada4c9fa.frag.glsl",
        "cs",
        36,
        4,
    ),
    ("shading:shading=cel", "shading-e5f21674435c0404.vert.glsl+shading-fea0d079a13967da.frag.glsl", "fs", 65, 4),
    ("shading:shading=cel", "shading-e5f21674435c0404.vert.glsl+shading-fea0d079a13967da.frag.glsl", "vs", 76, 4),
    ("shading:shading=cel", "shading-e5f21674435c0404.vert.glsl+shading-fea0d079a13967da.frag.glsl", "cs", 36, 4),
    ("shading:shading=gouraud", "shading-83cd81fbb702ee85.vert.glsl+build-474800a67afed087.frag.glsl", "fs", 9, 4),
    ("shading:shading=gouraud", "shading-83cd81fbb702ee85.vert.glsl+build-474800a67afed087.frag.glsl", "vs", 66, 4),
    ("shading:shading=gouraud", "shading-83cd81fbb702ee85.vert.glsl+build-474800a67afed087.frag.glsl", "cs", 36, 4),
    ("shading:shading=phong", "shading-e5f21674435c0404.vert.glsl+shading-8a641531f03295fb.frag.glsl", "fs", 60, 4),
    ("shading:shading=phong", "shading-e5f21674435c0404.vert.glsl+shading-8a641531f03295fb.frag.glsl", "vs", 81, 4),
    ("shading:shading=phong", "shading-e5f21674435c0404.vert.glsl+shading-8a641531f03295fb.frag.glsl", "cs", 36, 4),
    ("shadow", "shadow-37f46ae8961221f2.vert.glsl+shadow-928e172a68b7f175.frag.glsl", "fs", 8, 4),
    ("shadow", "shadow-37f46ae8961221f2.vert.glsl+shadow-928e172a68b7f175.frag.glsl", "vs", 43, 4),
    ("shadow", "shadow-37f46ae8961221f2.vert.glsl+shadow-928e172a68b7f175.frag.glsl", "cs", 36, 4),
    ("shadow", "shadow-6964fcf6aaac9739.vert.glsl+build-474800a67afed087.frag.glsl", "fs", 9, 4),
    ("shadow", "shadow-6964fcf6aaac9739.vert.glsl+build-474800a67afed087.frag.glsl", "vs", 68, 4),
    ("shadow", "shadow-6964fcf6aaac9739.vert.glsl+build-474800a67afed087.frag.glsl", "cs", 36, 4),
    ("shadow", "shadow-b9d18073e5979540.vert.glsl+build-474800a67afed087.frag.glsl", "fs", 9, 4),
    ("shadow", "shadow-b9d18073e5979540.vert.glsl+build-474800a67afed087.frag.glsl", "vs", 41, 4),
    ("shadow", "shadow-b9d18073e5979540.vert.glsl+build-474800a67afed087.frag.glsl", "cs", 36, 4),
    ("shadow", "shadow-b9d18073e5979540.vert.glsl+shadow-cf4bf899d84af04d.frag.glsl", "fs", 29, 4),
    ("shadow", "shadow-b9d18073e5979540.vert.glsl+shadow-cf4bf899d84af04d.frag.glsl", "vs", 60, 4),
    ("shadow", "shadow-b9d18073e5979540.vert.glsl+shadow-cf4bf899d84af04d.frag.glsl", "cs", 36, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-1637be2eef2e8a5e.frag.glsl", "fs", 1090, 2),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-1637be2eef2e8a5e.frag.glsl", "vs", 90, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-1637be2eef2e8a5e.frag.glsl", "cs", 87, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-4d266cda76461dc0.frag.glsl", "fs", 146, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-4d266cda76461dc0.frag.glsl", "vs", 154, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-4d266cda76461dc0.frag.glsl", "cs", 87, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-895a73bccb0398fc.frag.glsl", "fs", 14, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-895a73bccb0398fc.frag.glsl", "vs", 90, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-895a73bccb0398fc.frag.glsl", "cs", 87, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-8c8662245addb598.frag.glsl", "fs", 78, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-8c8662245addb598.frag.glsl", "vs", 90, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-8c8662245addb598.frag.glsl", "cs", 87, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-bbba1ac2cdc22b1a.frag.glsl", "fs", 78, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-bbba1ac2cdc22b1a.frag.glsl", "vs", 90, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-bbba1ac2cdc22b1a.frag.glsl", "cs", 87, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-bdfd8dd627aa0bf2.frag.glsl", "fs", 50, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-bdfd8dd627aa0bf2.frag.glsl", "vs", 90, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-bdfd8dd627aa0bf2.frag.glsl", "cs", 87, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-c549a79aadfd095b.frag.glsl", "fs", 35, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-c549a79aadfd095b.frag.glsl", "vs", 90, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-c549a79aadfd095b.frag.glsl", "cs", 87, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-d697bb5000d6955f.frag.glsl", "fs", 14, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-d697bb5000d6955f.frag.glsl", "vs", 90, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-d697bb5000d6955f.frag.glsl", "cs", 87, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-e043e1e89bacfac4.frag.glsl", "fs", 47, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-e043e1e89bacfac4.frag.glsl", "vs", 90, 4),
    ("terrain", "terrain-18bca803a331552c.vert.glsl+terrain-e043e1e89bacfac4.frag.glsl", "cs", 87, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-1637be2eef2e8a5e.frag.glsl", "fs", 1090, 2),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-1637be2eef2e8a5e.frag.glsl", "vs", 26, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-1637be2eef2e8a5e.frag.glsl", "cs", 20, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-895a73bccb0398fc.frag.glsl", "fs", 14, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-895a73bccb0398fc.frag.glsl", "vs", 26, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-895a73bccb0398fc.frag.glsl", "cs", 20, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-8c8662245addb598.frag.glsl", "fs", 78, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-8c8662245addb598.frag.glsl", "vs", 26, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-8c8662245addb598.frag.glsl", "cs", 20, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-bbba1ac2cdc22b1a.frag.glsl", "fs", 78, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-bbba1ac2cdc22b1a.frag.glsl", "vs", 26, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-bbba1ac2cdc22b1a.frag.glsl", "cs", 20, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-bdfd8dd627aa0bf2.frag.glsl", "fs", 50, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-bdfd8dd627aa0bf2.frag.glsl", "vs", 26, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-bdfd8dd627aa0bf2.frag.glsl", "cs", 20, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-c549a79aadfd095b.frag.glsl", "fs", 35, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-c549a79aadfd095b.frag.glsl", "vs", 26, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-c549a79aadfd095b.frag.glsl", "cs", 20, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-d697bb5000d6955f.frag.glsl", "fs", 14, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-d697bb5000d6955f.frag.glsl", "vs", 26, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-d697bb5000d6955f.frag.glsl", "cs", 20, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-e043e1e89bacfac4.frag.glsl", "fs", 47, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-e043e1e89bacfac4.frag.glsl", "vs", 26, 4),
    ("terrain", "terrain-8b696c5053d8c476.vert.glsl+terrain-e043e1e89bacfac4.frag.glsl", "cs", 20, 4),
    (
        "texture:texture-filter=linear",
        "build-ace204f6171a58b1.vert.glsl+texture-425bd493829699b6.frag.glsl",
        "fs",
        17,
        4,
    ),
    (
        "texture:texture-filter=linear",
        "build-ace204f6171a58b1.vert.glsl+texture-425bd493829699b6.frag.glsl",
        "vs",
        72,
        4,
    ),
    (
        "texture:texture-filter=linear",
        "build-ace204f6171a58b1.vert.glsl+texture-425bd493829699b6.frag.glsl",
        "cs",
        36,
        4,
    ),
    (
        "texture:texture-filter=mipmap",
        "build-ace204f6171a58b1.vert.glsl+texture-425bd493829699b6.frag.glsl",
        "fs",
        17,
        4,
    ),
    (
        "texture:texture-filter=mipmap",
        "build-ace204f6171a58b1.vert.glsl+texture-425bd493829699b6.frag.glsl",
        "vs",
        72,
        4,
    ),
    (
        "texture:texture-filter=mipmap",
        "build-ace204f6171a58b1.vert.glsl+texture-425bd493829699b6.frag.glsl",
        "cs",
        36,
        4,
    ),
    (
        "texture:texture-filter=nearest",
        "build-ace204f6171a58b1.vert.glsl+texture-425bd493829699b6.frag.glsl",
        "fs",
        17,
        4,
    ),
    (
        "texture:texture-filter=nearest",
        "build-ace204f6171a58b1.vert.glsl+texture-425bd493829699b6.frag.glsl",
        "vs",
        72,
        4,
    ),
    (
        "texture:texture-filter=nearest",
        "build-ace204f6171a58b1.vert.glsl+texture-425bd493829699b6.frag.glsl",
        "cs",
        36,
        4,
    ),
];

type Row = (String, String, &'static str, usize, u32);

fn measure(dir: &Path, index: &str) -> Vec<Row> {
    let mut by_entry: BTreeMap<String, (Vec<String>, Vec<String>)> = BTreeMap::new();
    for l in index.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let (f, entries) = l.split_once(": ").expect("`<file>: <entries>`");
        for e in entries.split("; ") {
            let slot = by_entry.entry(e.to_string()).or_default();
            if f.contains(".vert.") {
                slot.0.push(f.to_string());
            } else {
                slot.1.push(f.to_string());
            }
        }
    }
    let read = |n: &str| std::fs::read_to_string(dir.join(n)).expect("a shader file");
    let mut rows = Vec::new();
    for (entry, (vss, fss)) in &by_entry {
        for v in vss {
            for f in fss {
                let mut mem = vec![0u8; 64 << 20];
                let a = Arena::new(&mut mem);
                let vs = lower::compile_to_ir(&a, sema::Stage::Vertex, a.alloc_str(&read(v)).expect("arena"));
                let fs = lower::compile_to_ir(&a, sema::Stage::Fragment, a.alloc_str(&read(f)).expect("arena"));
                let (Ok(vs), Ok(fs)) = (vs, fs) else { panic!("{entry}: {v}+{f} does not compile") };
                let Ok(p) = compile_program(&vs.module, &fs.module, Prim::Triangles, TARGET, &a) else {
                    continue; // not a pair glmark2 links
                };
                let name = format!("{v}+{f}");
                rows.push((entry.clone(), name.clone(), "fs", p.fs.code.words.len(), p.fs.threads));
                rows.push((entry.clone(), name.clone(), "vs", p.vs.code.words.len(), p.vs.threads));
                rows.push((entry.clone(), name, "cs", p.cs.code.words.len(), p.cs.threads));
            }
        }
    }
    rows
}

#[test]
fn glmark2_programs_word_and_thread_counts() {
    let named = std::env::var_os("GLMARK2_CORPUS");
    let dir = match &named {
        Some(d) => PathBuf::from(d),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("build/glmark2-host/shaders"),
    };
    let Ok(index) = std::fs::read_to_string(dir.join("index.txt")) else {
        assert!(named.is_none(), "GLMARK2_CORPUS={} has no index.txt", dir.display());
        eprintln!(
            "SKIP v3d_glmark2_words: no corpus at {} (set GLMARK2_SRC and run tools/hostharness/glmark2/run.sh \"$GLMARK2_SRC\")",
            dir.display()
        );
        return;
    };
    let rows = measure(&dir, &index);
    println!("v3d 4.2 glmark2-es2 programs: {} rows (scene, program, stage, words, threads)", rows.len());
    for (s, p, st, w, t) in &rows {
        println!("PIN     ({s:?}, {p:?}, {st:?}, {w}, {t}),");
    }

    let want: BTreeMap<(&str, &str, &str), (usize, u32)> =
        PINS.iter().map(|&(s, p, st, w, t)| ((s, p, st), (w, t))).collect();
    assert_eq!(want.len(), PINS.len(), "PINS lists a row twice");
    let mut bad = Vec::new();
    let mut seen = 0;
    for (s, p, st, w, t) in &rows {
        match want.get(&(s.as_str(), p.as_str(), *st)) {
            None => bad.push(format!("{s} {p} {st}: not pinned (now {w} words, {t} threads)")),
            Some(&(pw, pt)) => {
                seen += 1;
                if (pw, pt) != (*w, *t) {
                    bad.push(format!(
                        "{s} {p} {st}: moved from {pw} words at {pt} threads to {w} words at {t} threads ({:+} words)",
                        *w as i64 - pw as i64
                    ));
                }
            }
        }
    }
    if seen != PINS.len() {
        bad.push(format!("{} pinned rows are no longer produced", PINS.len() - seen));
    }
    assert!(
        bad.is_empty(),
        "glmark2 V3D 4.2 counts moved. A scheduler or allocator change re-pins PINS in the same commit: \
         replace its rows with the `PIN ` lines of --nocapture.\n{}",
        bad.join("\n")
    );
}
