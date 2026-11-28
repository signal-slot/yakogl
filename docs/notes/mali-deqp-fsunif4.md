<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): mediump fragment-shader uniform shapes S3, S5 and S9

**Status: study** (plan #402, item `ref-fsunif4`; issue #20). Nothing under
`src/` changed. The references are `tools/mali/testdata/deqp_fsunif4_*`,
pinned by `tests/mali_deqp_fsunif4.rs`. Measured HOST-ONLY at the tip
**`c28470db`** (2026-11-28); nothing here was run on the NanoPC-T6. The
refusal and its earlier shapes are in `docs/notes/mali-deqp-fsunif.md` and
`docs/notes/mali-deqp-fsunif2.md`; the shape names S1..S12 are those of
plan #402's `triage-fsunif-tip` (at `349f989f`).

## 1. Measure

The 272 cases of `refusals.tsv` of
`tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055` with this refusal's
board text went through `tools/mali/deqp-host-measure.sh --cases`
(VK-GL-CTS `93bca01` with the yakogl target, `$CONNECTED_DEPS_DIR/VK-GL-CTS-host`;
the tree a copy of `c28470db` with the duplicate `vivante-st` key at
`capi/Cargo.toml:85` deleted there only). 88 of them refuse with this text
first on the host, in the fragment shader. `random.scalar_conversion.combined.94`
(board: the VS uniform-type refusal) was measured alone and refuses with
this text too.

Each of their programs went through `mali_ref::compile_pair` in a second
scratch copy whose catch-all arm of `vectorise`
(`src/shaderc/online/mali/nir_fs.rs:11167`, `(Some(_), ..) | (_, Some(_), ..)
=> return Err(MaliError::FsUniform)`) printed the pair's place (colour x,y,
colour z,w or an operation's operand) and the kind of each lane (uniform
half, constant, other lane). 48 cases refuse there; 40 refuse at other
lines: 32 `shaders.matrix.*` (S1, S6), 6 `shaders.struct.uniform.*` (S7, S8),
`random.conditionals.fragment.61` (S10) and `uniform_api.random.23` (S12),
which existing references cover, or which have none yet (S10) or cannot have
one (S12).

## 2. Shapes at `nir_fs.rs:11167`

| Shape | Place | First lane | Second lane | Cases | References |
|---|---|---|---|--:|---|
| S2 | operand | uniform half | constant | 18 | `deqp_intval3_rsg_swizzle12_fs`, `deqp_unifmp_rsg_expr10` (earlier) |
| **S3** | operand | constant | uniform half | 21 | **`deqp_fsunif4_s3_conv_fs_4`, `_s3_expr_fs_20`, `_s3_interaction_30`** |
| S4 | operand | uniform half | other lane | 5 | `deqp_unif2_fs_other` (earlier) |
| **S5** | operand | other lane | uniform half | 3 | **`deqp_fsunif4_s5_expr_fs_97`, `_s5_expr_fs_74`, `_s5_conv_fs_63`** |
| **S9** | colour x,y | constant | uniform half | 2 | **`deqp_fsunif4_s9_conv_fs_94`, `_s9_conv_combined_94`** |

The triage at `349f989f` counted S3 24, S5 5 and S9 2: it measured the whole
tree, so it also counted cases whose board refusal was another one and that
reach this one now (`combined.94` is such a case for S9). Here S3 and S5 are
counted over this refusal's board cases only.

## 3. The cases

S3 (21): `fragment_ops.interaction.basic_shader.30`;
`shaders.random.basic_expression.{combined.14, combined.20, combined.36,
combined.44, fragment.14, fragment.20, fragment.24, fragment.36,
fragment.44}`; `shaders.random.conditionals.{combined.26, fragment.26}`;
`shaders.random.comparison_ops.fragment.4`;
`shaders.random.scalar_conversion.{combined.13, combined.34, combined.99,
fragment.4, fragment.13, fragment.34, fragment.99}`;
`shaders.random.swizzle.fragment.30`.

S5 (3): `shaders.random.basic_expression.{fragment.74, fragment.97}`,
`shaders.random.scalar_conversion.fragment.63`.

S9 (2): `shaders.random.scalar_conversion.{fragment.94, combined.94}`.

The eight references (at most eight, three per shape, two for S9) are:

| Reference | Case | The pair |
|---|---|---|
| `s3_conv_fs_4` | `random.scalar_conversion.fragment.4` | `vec4(c, b, 0.375, 5.0) * vec4(..)`: constant `c`, uniform `b` |
| `s3_expr_fs_20` | `random.basic_expression.fragment.20` | `c + vec4(1.5, b, b, a)` |
| `s3_interaction_30` | `fragment_ops.interaction.basic_shader.30` | `vec4(e, d, c, b) - vec4(..)`: constant `e`, uniform `d` |
| `s5_expr_fs_97` | `random.basic_expression.fragment.97` | `vec4(b, d, e, d) * vec4(..)`: varying `b`, uniform `d` |
| `s5_expr_fs_74` | `random.basic_expression.fragment.74` | `e * d * vec4(c, -1.0, b, a)`: a computed lane beside uniform `c` |
| `s5_conv_fs_63` | `random.scalar_conversion.fragment.63` | `vec4(g, f, e, 0.0) * vec4(..)`: varying `g`, uniform `f` |
| `s9_conv_fs_94` | `random.scalar_conversion.fragment.94` | `gl_FragColor = vec4(bool(c), a, b, a)`: 0.0 and uniform `a` as the colour's first word |
| `s9_conv_combined_94` | `random.scalar_conversion.combined.94` | the same fragment shader beside a random vertex shader |

All were drawn with `tools/mali/deqp_ref.sh` (shim instrument, mediump,
`--uniform-values`) on Mesa 26.2.1's drm-shim (`$CONNECTED_DEPS_DIR/mesa-shim`),
with their `.nir.sha` from `tools/mali/nir_refs.sh`. The sources are verbatim
from the measure's `.qpa` (each `.license` names its case).

## 4. Where the lowering goes

All three are new arms of `vectorise` before the catch-all at
`nir_fs.rs:11167` (line numbers at `c28470db`):

- S9 mirrors the colour-z,w arm at `nir_fs.rs:11145`
  (`(None, Some(_), Val::Const(c), _)` guarded by `(a, b) == (w[2], w[3])`,
  `Leaf::Lane { half, first: true }`) for colour x,y.
- S3 is the same constant-first pair as an operation's operand; S2, its
  reverse order (uniform half, then a constant), has the references named
  above, and the lowering should take the constant-first order as the
  `MKVEC.v2i16` Mesa writes in these references.
- S5 is the reverse order of S4 as an operand; the colour-word arm at
  `nir_fs.rs:11159` (`Leaf::Pack` of the two lanes, either order) is the one
  to extend to operands, as S4's.

A lowering that takes one removes its `#[ignore]`, moves its entry from
`REFUSED` to `MATCHED` in `tests/mali_deqp_fsunif4.rs`, and moves the pin.
