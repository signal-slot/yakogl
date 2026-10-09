# glmark2's shaders

The distinct shader texts glmark2-es2's default benchmark set passes to
`glShaderSource`. They are **not committed**: glmark2 is GPL-3.0-or-later and
this repository is Apache-2.0 (`driver/docs/glmark2.md`; upstream is likewise
fetched, never copied, by `driver/samples/zephyr/glmark2/CMakeLists.txt`). The
corpus is produced from a glmark2 checkout on demand.

* **Source:** glmark2 (<https://github.com/glmark2/glmark2>) at commit
  `22c527cb0556f3a1ac4445aaa52cc532760928d5`, the commit the Zephyr image
  pins. `GLMARK2_SRC` is a git checkout of it (verified) or the GitHub archive
  directory `glmark2-<commit>`.
* **Generation:** `tools/hostharness/glmark2/run.sh "$GLMARK2_SRC"` runs the
  default benchmark set on the host against YakoGL
  ([`tools/hostharness/glmark2/`](../../hostharness/glmark2/README.md)),
  captures every text glmark2 passed (after glmark2's own preprocessing, which
  adds its defines, constants and loop bodies per scene option), and writes
  them to `build/glmark2-host/shaders/` (gitignored). Each file starts with a
  `// SPDX-License-Identifier: GPL-3.0-only` line and one provenance comment,
  then the text unchanged.
* **Names:** `<scene>-<fnv1a64 of the text>.<vert|frag>.glsl`. The scene is
  the first default-list entry that compiled the text. `index.txt` lists
  every entry that did.

The extension is `.glsl`, not `.vert`/`.frag`, so the corpus walkers
(`tests/online_corpus.rs`, `tests/stack_survey.rs`) never pick the files up.

## Compile test

`tests/glmark2_corpus.rs` compiles every file in `index.txt` through the
online compiler (`src/shaderc/online`) of the build's GPU and, per index
entry, links the vertex/fragment pairs (the index does not record which
vertex shader went with which fragment shader inside one entry, so every pair
of the entry is tried and each shader must link with at least one partner).
Every shader must compile and link; the index is pinned at 69 shaders.

It reads `GLMARK2_CORPUS` (a directory with `index.txt`), by default
`build/glmark2-host/shaders`. With no corpus it prints a `SKIP` line and
returns; a directory `GLMARK2_CORPUS` names must hold it, or the test fails. `tools/check.sh`
generates the corpus from `GLMARK2_SRC` and runs the three back ends; without
`GLMARK2_SRC` that row is a gap, so the gate is red.

```text
GM2_WRITE=0 tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-host powervr vivante
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test glmark2_corpus -- --nocapture
cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test glmark2_corpus -- --nocapture
cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test glmark2_corpus -- --nocapture
```

`--nocapture` prints the per-back-end counts and every failure with the
compiler's message.
