<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the `Defect(IrIndex)` shapes U1 and U2 and their `deqp_irindex4` references

**HOST-ONLY.** Plan #402 item `ref-irindex4`. The tip is **`c28470db`** and
every line number below is that commit's. The item adds no compiler change. It
adds the references `tools/mali/testdata/deqp_irindex4_*` and
`tests/mali_deqp_irindex4.rs`. This note adds to
`docs/notes/mali-deqp-irindex.md`, whose section 5 names root causes 5.1 to
5.8, and to `docs/notes/mali-deqp-irindex23.md`. It does not edit them.

## Method

- **Sources.** This machine had no host dEQP build, so no `.qpa` was
  available. The shaders were written by small scripts that follow dEQP's
  generators in VK-GL-CTS `05ca62d` (fetched shallow and sparse into
  `$CONNECTED_DEPS_DIR/VK-GL-CTS-src`):
  - `ShaderMatrixCase::init` (`modules/gles2/functional/es2fShaderMatrixTests.cpp`)
  - `ShaderOperatorCase` (`es2fShaderOperatorTests.cpp`)
  - the shader library's `specializeFragmentShader` and `genVertexShader`
    (`modules/glshared/glsShaderLibraryCase.cpp`) for the `.test` files

  The library script reproduces `deqp_irindex_b2f16_pair.{vert,frag}`, which
  was taken from a `.qpa`, byte for byte.
- **Scan.** Every pair was compiled with `tests/mali_ref::compile_pair` on a
  scratch copy of the tip. On that copy `capi/Cargo.toml` line 85, the
  duplicate `vivante-st`, was deleted. The pairs scanned were:
  - every IrIndex `conversions.*`, `swizzles.*`, `functions.datatypes.*` and
    `preprocessor.*` fragment case of the 8ff01055 slices
    (`tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/refusals.tsv`)
  - all 279 `matrix.*` fragment cases of the `uniform` and `dynamic` inputs
  - the 147 fragment cases of `operator.{float,int,bool}_compare.*` and
    `binary_operator.{equal,not_equal}.*`
- **Sites.** Each `Defect(IrIndex)` construction of `src/shaderc/online/mali`
  was then made to print its `file!():line!()`, `index` printed its
  `#[track_caller]` caller, and `src32` printed which of its `ok_or(defect)`
  calls failed. A run with `CARGO_PROFILE_TEST_DEBUG=line-tables-only` added a
  backtrace. Many constructions are built eagerly and never returned (for
  example `mod.rs:1950` and `:2008`, and `src32`'s `let defect` at
  `nir_fs.rs:13485`), so only the failing call names the site. All results
  were re-run on a clean, uninstrumented build.

## What is IrIndex at the tip

| Cases at `c28470db` (HOST-ONLY) | Site | Shape |
|---|---|---|
| `matrix.div.dynamic_{lowp,mediump}_mat4_mat4`, `matrix.div_assign.dynamic_{lowp,mediump}_mat4` (4) | `nir_fs.rs:11014` | **U1** |
| `operator.bool_compare.not.{bvec3,bvec4}` (2) | `nir_fs.rs:13498` | **U2** |
| `matrix.{add,sub}.dynamic_{lowp,mediump}_mat4_{float,mat4}`, `{add,sub}_assign`, `{pre,post}_{in,de}crement` of `dynamic_{lowp,mediump}_mat4` (20) | `nir_fs.rs:10491` (`split_lanes`) | irindex note 5.2, `deqp_irindex_mat4_pair_unread` |
| `conversions.matrix_to_matrix.mat3_to_{mat2,mat3}`, `vector_combine.vec3_vec3_vec3_to_mat3`, `functions.datatypes.float_mat3` | `src32` raw load | irindex note 5.3, `deqp_irindex_load_mat3` |

The 8ff01055 slices recorded the four U1 cases as "arithmetic other than
+, -, * ...". The fragment `/` has been lowered since then, so they now reach
U1. The U2 cases were already IrIndex there. At the tip, all 108 IrIndex
`swizzles.vector_swizzles` cases of the slices compile, and so do the bvec
conversions that 5.4 (`b2f16_pair`) fixed.

## U1: a `?` in a `vectorise` match guard fails the shader

- **Site.** `vectorise`, `nir_fs.rs:11014`:

  ```rust
  (Some((v, i)), Some((w, j)), ..) if index(f, v, i)?.0 == index(f, w, j)?.0 => {
  ```

  The full chain is `index` (`:10146`, `lane_of` has no entry), called from
  that guard, `vectorise`'s product arm (`:10965`, the two operands of an
  `fmul`), `pair_node` (`:11791`), `lane_tree` (`:11822`, `:11982`,
  `:11983`), `vectorise` (`:10805`) and `select` (`:12448`).
- **Root cause.** The colour pairs two lanes of a product whose varying
  operand's lane has no `lane_of` entry: the swizzle reads `.zwx` of the
  product, so lane `y` of the varying is read nowhere. The `?` in the guard
  then returns `Defect(IrIndex)` for the whole shader, where it should only
  make this arm not apply. This is the same invariant as 5.2, whose fix
  direction for `split_lanes` is "a lane with no `lane_of` entry: answer
  `Ok(false)`". A `/` by a uniform is one of these products: Mesa lowers it to
  `m * rcp(u)`.
- **The next guard and the arm below.** On a scratch copy with this guard
  changed to
  `matches!((index(f, v, i), index(f, w, j)), (Ok(x), Ok(y)) if x.0 == y.0)`,
  the same cases fail two places further down:
  - the guard at `:11085`, which has the same `?`s
  - the body of the `Leaf::Lanes` arm (`:11076`, `index(f, w, j)?`)

  With all three made not to apply, the four dEQP cases reach the named
  refusal "a gl_FragColor lane other than the varying's own lane or a
  constant is not lowered yet". The fix therefore needs the pairing that
  `deqp_irindex4_u1_*` draws, not only the guards.
- **Minimal pair** (`tests/mali_deqp_irindex4.rs`, `MINIMAL`). The vertex
  shader writes `varying mediump vec4 m`, and the two shaders differ in one
  swizzle:

  ```glsl
  precision mediump float; varying vec4 m; uniform vec4 u;
  void main() { gl_FragColor = vec4((m * u).zwx, 1.0); }   // Defect(IrIndex), nir_fs.rs:11014
  void main() { gl_FragColor = vec4((m * u).xyz, 1.0); }   // compiles
  ```

  `(m * m).zwx`, `(m / u).zwx` and `(m * u).zwy` hit it too. In
  `(m * u).zwy` the first `index` of the guard fails, not the second. The
  sum `(m + u).zwx` fails in `split_lanes` instead (5.2).
- **References.**
  - `deqp_irindex4_u1_div_mediump` is `matrix.div.dynamic_mediump_mat4_mat4_fragment`.
  - `deqp_irindex4_u1_divassign_mediump` is `matrix.div_assign.dynamic_mediump_mat4_fragment`.

  Mesa draws the same words for both. The lowp cases draw the same words as
  the mediump ones, apart from the precision names in the source, so they
  have no reference.

## U2: the value of a 16-bit `B2f` lane the colour reads is missing

- **Site.** `src32`'s 16-bit arm, `nir_fs.rs:13498`:

  ```rust
  Kind::Alu(_) if f.nir.nodes[n as usize].bits == 16 => {
      let s = val.get(n as usize).copied().flatten().ok_or(defect)?;
  ```

  `defect` is built at `:13485`. The node with no value is `Alu(B2f)` with
  `bits 16`, `width 1`. These are the float conversions of the colour's
  boolean lanes.
- **Root cause.** The colour reads two `float(bool)` lanes as one pair, which
  is a two-lane `B2f.16`. The trace shows that `src32` asks for a one-lane
  `B2f` node's 16-bit half and finds no `val[]` entry for it. The likely cause
  is that the pair's two-lane operation sets no value for the one-lane nodes
  it stands for. The trace does not show which pass should set it. A single `float(bool)` lane compiles.
  dEQP's `not.bvec2` writes `color.yz` and compiles, while `vec4(vec2(b), 0.0,
  1.0)` and `vec4(0.0, vec2(b), 1.0)` of a `bvec2` do not.
- **Minimal pair** (`MINIMAL`). The two shaders differ in whether the colour
  reads one boolean lane or two:

  ```glsl
  precision mediump float; varying vec4 m;
  void main() { bool b = m.x > 0.0; bool c = m.y > 0.0;
                gl_FragColor = vec4(float(b), float(c), 0.0, 1.0); }    // Defect(IrIndex), nir_fs.rs:13498
  void main() { bool b = m.x > 0.0;
                gl_FragColor = vec4(float(b), 0.0, 0.0, 1.0); }         // compiles
  ```

  `vec4(vec3(greaterThan(m.xyz, vec3(0.0))), 1.0)`, its `lessThanEqual`
  form and its `not(...)` form hit it too.
- **References.**
  - `deqp_irindex4_u2_not_bvec3` is `operator.bool_compare.not.bvec3_fragment`.
  - `deqp_irindex4_u2_not_bvec4` is `operator.bool_compare.not.bvec4_fragment`.
- **Fix direction.** Give the one-lane node the half of the pair it is in, as
  5.4's fix gave the two-lane `B2f.16` its `MUX.v2i16`, or emit the pair's
  lanes as the reference draws them. Then check the result against the two
  references.

## Caveats

- The sources are reconstructed, not taken from a `.qpa`. An item with a host
  dEQP build (`tools/mali/deqp-host-measure.sh`) should confirm them.
- The scan covers the generated families named above and the IrIndex cases of
  the 8ff01055 slices. It is not a whole-tree run at the tip. A case that is
  IrIndex only at the tip, in another family, could still be missing.
- `deqp_irindex3_*` was written by hand with two mat4 varyings. dEQP's
  `dynamic_*_mat4_mat4` cases have one mat4 varying and a uniform mat4
  (`ShaderMatrixCase::init` allows only one dynamic matrix input). These
  references follow dEQP.
