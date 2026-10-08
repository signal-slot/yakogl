# glmark2's shaders

The distinct shader texts glmark2-es2's default benchmark set passes to
`glShaderSource`, captured by the host run
([`tools/hostharness/glmark2/`](../../hostharness/glmark2/README.md)).

* **Source:** glmark2 (<https://github.com/glmark2/glmark2>) at commit
  `22c527cb0556f3a1ac4445aaa52cc532760928d5`, the commit the Zephyr image
  pins (`driver/samples/zephyr/glmark2/CMakeLists.txt`). These are glmark2's
  `data/shaders/` files after glmark2's own preprocessing, which adds its
  defines, constants and loop bodies per scene option, exactly as the driver
  receives them.
* **Licence:** glmark2 is GPL-3.0-or-later. Each file starts with
  `// SPDX-License-Identifier: GPL-3.0-only` (see `LICENSES/GPL-3.0-only.txt`)
  and one provenance comment line, then the text glmark2 passed, unchanged.
  Both lines are comments, which GLSL ES 1.00 allows before `#version`.
* **Names:** `<scene>-<fnv1a64 of the text>.<vert|frag>.glsl`. The scene is
  the first default-list entry that compiled the text. `index.txt` lists
  every entry that did.
* **Regeneration:** `tools/hostharness/glmark2/run.sh "$GLMARK2_SRC"`
  rewrites every `.glsl` file and `index.txt` here. This README is not
  rewritten.

The extension is `.glsl`, not `.vert`/`.frag`. The corpus walkers
(`tests/online_corpus.rs`, `tests/stack_survey.rs`) compile every `.vert` and
`.frag` under `tools/shaders/`, and `online_corpus.rs` pins that population
(734) and its refusal fixtures. Adding these to that population means
re-deriving the pin, which belongs to that test's owner (glmark2-corpus).
Until then these files are outside it. The stage is the second-to-last
extension.

## Compile test

`tests/glmark2_corpus.rs` compiles every file in `index.txt` through the
online compiler (`src/shaderc/online`) of the build's GPU and, per index
entry, links the vertex/fragment pairs (the index does not record which
vertex shader went with which fragment shader inside one entry, so every pair
of the entry is tried and each shader must link with at least one partner).
It runs per back end, like `tests/online_corpus.rs`:

```text
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test glmark2_corpus -- --nocapture
cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test glmark2_corpus -- --nocapture
cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test glmark2_corpus -- --nocapture
```

`--nocapture` prints the per-back-end counts and every failure with the
compiler's message. Today: 69 of 69 compile and link on PowerVR, GC7000 and
V3D.

`known-uncompilable.txt` lists the expected failures, one
`<shader file> <powervr|gc7000|v3d> <reason>` per line (`#` lines are
comments). A shader not listed that fails fails the test, and so does a listed
one that now compiles (a stale entry). It is empty, and should stay that way
unless a compiler gap is left for a later fix. The test also pins the index at
69 shaders; re-derive that after a regeneration.
