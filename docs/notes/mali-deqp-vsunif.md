<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali dEQP-GLES2: the vertex shader's "uniform other than a float, vec2, vec3, vec4 or mat4" (vsunif)

Plan #402 item `ref-vsunif` (issue #20). It takes one compile refusal from
the whole-tree NanoPC-T6 run at `8ff01055` (`docs/es2-cts-failures.md`
section M), `MaliError::UniformType`:

> Valhall: a uniform other than a float, vec2, vec3, vec4 or mat4 is not lowered yet

It grew from 362 Fail at `37305608` to 371 Fail (plus 92 NotSupported) at
`8ff01055`. This item makes references only; it changes no code. The
references are `tools/mali/testdata/deqp_vsunif_*` and their test is
`tests/mali_deqp_vsunif.rs`. Every count here is **HOST-ONLY**: a compile
and link outcome measured on this host. None of it was run on the board.

## 1. The measure

The case list is every case whose `refusals.tsv` text (column 2, the first
refusal, possibly cut) in the four slices
`tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/` holds the refusal:
463 cases (371 Fail and 92 NotSupported on the board). They were run at the
tip `e65567ea`, VK-GL-CTS `93bca01861b0`:

```sh
grep -hF 'a uniform other than a float, vec2, vec3, vec4 or mat4' \
    tools/cts/log/2026-11-24-t6-gles2-S{1,2,3,4}-8ff01055/refusals.tsv | cut -f1 | sort -u > "$TMPDIR/vsunif-cases.txt"
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS-host-gles2" \
    --slice tools/cts/log/2026-11-24-t6-gles2-S1-8ff01055 --slice tools/cts/log/2026-11-24-t6-gles2-S2-8ff01055 \
    --slice tools/cts/log/2026-11-24-t6-gles2-S3-8ff01055 --slice tools/cts/log/2026-11-24-t6-gles2-S4-8ff01055 \
    --cases "$TMPDIR/vsunif-cases.txt" --out "$TMPDIR/measure-vsunif"
```

`--filter` matches case names, not refusal text, so the refusal is selected
by the `--cases` list. The CTS tree is a shallow clone of the pinned commit
with the yakogl target patch and `external/fetch_sources.py`. It has a
sparse checkout that leaves out only `doc/`: the top-level CMakeLists needs
the `external/vulkancts` and `external/openglcts` directories even for
`deqp-gles2` alone.

All 463 cases began on the host. Five processes exited 1 and were resumed.
On the host:

| outcome | cases |
|---|---:|
| first refusal is still this one (all vertex stage) | **375** (284 Fail, 91 NS on the board) |
| another first refusal (#367's ARITH 21, CONST_FOLD 5, FMA 4, CONTROL_FLOW 3; varylink 3; constop 2; ATTR_FOLD 1; UniformPrecision 12; FsUniform 2) | 53 |
| no compile refusal (dEQP Fail 34, Pass 1) | 35 |

The measure's own two shapes for the 375 cases are `vs_int_uniform` 264
(182 F + 82 NS) and `vs_struct_uniform` 111 (102 F + 9 NS). This note splits
them by the source of the refusing vertex shader.

## 2. Where the compiler refuses

There are two sites, both in the vertex walk (`src/shaderc/online/mali/nir.rs`
at `e65567ea`):

- `uniform_of` (`nir.rs:1122`, the `_ =>` arm). It takes float, vec2..4,
  mat2..4, bvec2..4, int, uint, bool, ivec and uvec. Every vertex shader of
  the 375 cases declares a non-float uniform of that list (an `int` in all
  of them). A few also declare a sampler (`random_dead_int`'s 7
  `shaders.random.texture.vertex` cases) or an ivec. Which of the two sites
  refused each case was not traced. The eight references below were
  checked: all of them have an `int` read of the second site's kind.
- `IrOp::LoadUniform` (`nir.rs:5249-5257`). A `GL_INT` uniform (a plain one
  or a struct member) whose IR value is read other than as a uniform
  array's index (`index_only`, `nir.rs:1195`), a loop head's compare
  operand (`int_compare_only`, `nir.rs:1232`) or a conversion to float
  (`int_float_only`, `nir.rs:1211`).

Every shape below is an `int` read in a form those three predicates do not
take. In `random_dead_int`, the read is in dead code.

## 3. Shapes (HOST-ONLY, at `e65567ea`)

Cases are the 375 still refused by this text. Fail and NS are the board's
status at `8ff01055`. "Covered" means an existing reference has the shape's
form, and its pin stays in its own test.

| shape | cases | Fail | NS | example case (`dEQP-GLES2.functional.`) | reference |
|---|---:|---:|---:|---|---|
| `random_dead_int` | 111 | 111 | 0 | `shaders.random.basic_expression.vertex.13` | new `deqp_vsunif_random_dead_int` |
| `uapi_struct_int` | 79 | 79 | 0 | `uniform_api.value.assigned.by_value.render.array_in_struct.int_ivec4_both` | covered by `deqp_unif2_vs_struct_uniform` and `deqp_vs16ref_uapi_struct_int_ivec4` |
| `loop_bound_product` | 44 | 44 | 0 | `shaders.loops.for_dynamic_iterations.basic_mediump_int_vertex` | covered by `deqp_loops2_vs_for_dyn_nested` |
| `vec_subscript` | 27 | 0 | 27 | `shaders.indexing.vector_subscript.vec4_direct_write_dynamic_subscript_read_vertex` | new `deqp_vsunif_vec_subscript` |
| `struct_local` | 26 | 19 | 7 | `shaders.struct.local.basic_vertex` | new `deqp_vsunif_struct_local` |
| `tmp_array` | 24 | 0 | 24 | `shaders.indexing.tmp_array.vec4_static_write_dynamic_read_vertex` | new `deqp_vsunif_tmp_array` |
| `varying_array` | 16 | 0 | 16 | `shaders.indexing.varying_array.vec4_dynamic_write_static_read` | new `deqp_vsunif_varying_array` |
| `matrix_subscript` | 15 | 0 | 15 | `shaders.indexing.matrix_subscript.mat2_dynamic_write_dynamic_loop_read_vertex` | covered by `deqp_unif2_vs_int_uniform` (the same pair) |
| `uapi_int` | 15 | 15 | 0 | `uniform_api.value.initial.render.basic.int_both` | covered by `deqp_vs16ref_uapi_int` (the same vertex shader) |
| `loop_select` | 6 | 6 | 0 | `shaders.loops.for_uniform_iterations.select_iteration_count_vertex` | covered by `deqp_loops_vs_select` (the same pair) |
| `loop_vector_counter` | 6 | 6 | 0 | `shaders.loops.for_uniform_iterations.vector_counter_vertex` | covered by `deqp_loops_vs_vector` (the same pair) |
| `struct_cmp` | 2 | 2 | 0 | `shaders.struct.uniform.equal_vertex` | new `deqp_vsunif_struct_cmp` |
| `struct_dynidx` | 2 | 0 | 2 | `shaders.struct.uniform.struct_array_dynamic_index_vertex` | new `deqp_vsunif_struct_dynidx` |
| `struct_int` | 2 | 2 | 0 | `shaders.struct.uniform.nested_vertex` | new `deqp_vsunif_struct_int` |
| **total** | **375** | **284** | **91** | | 8 new, 6 covered |

How the cases were sorted. The refusing vertex shader is the one whose
info log in the `.qpa` holds the refusal. The dEQP group stands for its
form, and each group's form was checked by reading its sources:

- `random_dead_int`: every `shaders.random.*` and
  `fragment_ops.interaction.basic_shader.*` case. These generated shaders
  read a `mediump int` uniform (sometimes beside a sampler or an ivec) in
  int arithmetic, conversions and compares. In the sources read, every such
  read feeds only dead values: a local never stored, a `v_color` assigned
  again later. Mesa removes them, and in the reference
  `glGetUniformLocation("h")` is -1. Whether all 111 are dead was not
  checked one by one; a lowering should redraw a few more.
- `uapi_struct_int`: the `uniform_api` and `state_query` cases with a struct
  uniform whose `int`/`ivec4` members are compared to constants
  (`compare_int(u_var.m0, 6)`). The struct layouts are basic, array member,
  nested, struct array and unused members. Their compare form is
  `deqp_unif2_vs_struct_uniform`'s, and the basic struct is
  `deqp_vs16ref_uapi_struct_int_ivec4`. Both pin this refusal.
- `uapi_int`: an `int` uniform compared in a select
  (`a == b ? 1.0 : 0.0`), `deqp_vs16ref_uapi_int`'s form.
- `loop_bound_product`: `*_dynamic_iterations` loops, where the bound is an
  int product `ui_n * one` or `one * ui_n` (`one = int(a_one + 0.5)`).
  `deqp_loops2_vs_for_dyn_nested` pins this refusal for that form.
- `vec_subscript`, `tmp_array`, `matrix_subscript`: an int uniform as the
  run-time index of a local vector, a local array or a local matrix.
- `varying_array`: an int uniform as the run-time index of a write to a
  varying array (`var[ui_zero] = ...`).
- `struct_local`: an int uniform stored into a local struct's int member
  (`S s = S(.., ui_one)`, then `float(s.c)`), or used as a local struct
  array's run-time index. The 7 NS cases with `dynamic` in their name
  (`*_dynamic_index`, `dynamic_loop_*`) are the second form, or a dynamic
  loop over the struct array. The representative is the first form.
- `struct_cmp`, `struct_int`, `struct_dynidx`: `shaders.struct.uniform`.
  These are whole struct uniforms compared (`a == b`, an `int c` member);
  int members added (`s.b.a + s.c`, `s[2].b - s[1].b + s[0].b`); and a
  struct uniform array read at int uniform indices.

The 23 pairs of `uapi_struct_int` differ only in layout and constants.
None of the other covered shapes needs a second copy.

## 4. The references

Each is the representative case's refusing pair, verbatim from the `.qpa`
(each source ends in a newline). It was drawn on Mesa 26.2.1's drm-shim
(`$CONNECTED_DEPS_DIR/mesa-shim`, `tools/mali/drm_shim_ref.sh build` with
`LLVM_DIR=/usr/lib/llvm-21`):

```sh
tools/mali/deqp_ref.sh <case>.vert <case>.frag tools/mali/testdata/deqp_vsunif_<shape> --instrument shim
```

As a self-check, this shim redrew `deqp_unif2_vs_int_uniform` and
`deqp_unif2_vs_struct_uniform`. Their regions are identical. Only the
`attributes` fields are absent, because those two were drawn with
`pan_ref_diff.py --attributes`, which `deqp_ref.sh` does not pass. The draws
set every uniform (`--uniform-values`: value index k gives 16 (k + 1) + c),
struct members one by one. The struct references (`struct_cmp`,
`struct_int`, `struct_dynidx`) are therefore compared with their members'
values. `tests/mali_deqp_vsunif.rs` calls `mali_ref::differences_members(..,
true)` for them, because they are not in `mali_ref::STRUCT_VALUES`.

At `e65567ea` all eight are refused with exactly the refusal above. They
are pinned in `refusals_pinned`, and their byte-compare tests are
`#[ignore]`d (lowered by plan #402's `lower-vsunif`).

## 5. What Mesa's program does that the compiler does not yet

These are the vertex varying programs (`vs.varying`) of the references. In
each, the position program is the usual attribute load and viewport
transform.

- **`random_dead_int`**. There is no uniform read at all. The program is
  `LD_ATTR_IMM.v4` of `c`, four `FADD_IMM.f32` of the folded constant
  `b * vec4(a, ..)` and the stores. The FAU holds only the varying
  offsets. The compiler must drop the dead int reads (and their locals)
  before the `LoadUniform` check, as Mesa's DCE does. It must also leave
  the uniform out of the block, since Mesa keeps no slot for it.
- **`vec_subscript`**. `tmp[ui_k]` of a mediump vec4 is a bcsel tree. For
  each read there are three `ICMP_OR.s32.lt.m1` of the int word against
  byte constants (`0x3020100.b3`, `0x1000000.b3`, `0x3020100.b2`, so
  `< 3`, `< 1`, `< 2`), and three `MUX.v2i16` over the fp16 lanes of the
  value (`V2F32_TO_V2F16` of the attribute, `FMA.v2f16` by the fp16
  constant pair). Then `FADD.v2f16` of the four reads, one `F16_TO_F32`
  and the same word stored to the three lanes of `v_color`. The int words
  are FAU `u5.w1`..`u7.w0`, read raw.
- **`tmp_array`**. A local `vec4 arr[4]` at run-time indices is the same
  tree, `nir_lower_indirect_derefs`, with two `MUX.v2i16` per compare,
  over the four elements' fp16 pairs. The `0.5` factor is from the FAU
  (`u7.w1`, fp16 `0x3000`), the others are immediates. Then two
  `FADD.v2f16` per step, `F16_TO_F32` of each lane, and the stores with
  `IADD_IMM` offsets.
- **`varying_array`**. Here the varying array's write index is dynamic.
  Each write `var[ui_k] = x` becomes, for each of the four elements, a
  `CSEL.u32.eq` per lane on the mask of `index == element`. The masks are
  built with `ICMP_OR.s32.lt.m1` against the byte constants and combined
  with `LSHIFT_OR`/`LSHIFT_AND` (`.not`), at fp32 (`FMA.f32` by
  `0x38000000.h1` and the FAU's 0.125). Sixteen varying lanes are stored
  with FAU offsets up to `u13.w0`.
- **`struct_local`**. The local struct is copy-propagated away. `s.c =
  ui_one` then `float(s.c)` is `S32_TO_F32 r2, u5.w1`, the int word
  converted in place, which is the `int_float_only` form. The compiler's
  read goes through a `StoreLocal`/`LoadLocal` pair, so the predicate does
  not see it. The `*_dynamic_index` cases of the shape also need the bcsel
  tree of `tmp_array`.
- **`struct_cmp`**. `a == b` of a struct `{float a; vec3 b; int c;}` is a
  member-wise AND. The mediump float and vec3 members are narrowed to fp16
  pairs (`V2F32_TO_V2F16`, `MKVEC.v2i16`) and compared with
  `FCMP_OR.v2f16.eq.m1`, the odd lane with `FCMP_OR.f32.eq.m1`, and the
  `int` member with `ICMP_OR.u32.eq.m1`. The masks are joined with
  `LSHIFT_AND.i32` (`S16_TO_S32` of the fp16 masks) and turned into the
  colour lane with `CSEL.u32.eq .., 0x0, 0x3F800000`. `a == d` with the
  local `d = S(uf_one, vec3(0.0, floor(..), 2.0), ui_two)` compares
  against the uniforms `uf_one` and `ui_two` in place.
- **`struct_int`**. `s.b.a + s.c` is `IADD.s32 r2, u5.w1, u6.w0` (after a
  `MOV` of one word), then `S32_TO_F32` and the mediump varying's
  `FMA.f32 x, 0, x`. That last step is the flush every float lane of this
  program gets (`s.a`, `s.b.b`). The compiler needs an int add of two FAU
  words before the conversion, beyond `int_float_only`'s direct
  conversion.
- **`struct_dynidx`**. `s[ui_k].a` and `.b` of `uniform S s[3]` are not
  FAU reads. Each one is a push-constant load at a computed offset:
  `LSHIFT_OR.i32` of the int word by `0x3020100.b33` (index × 8, the
  struct's stride in bytes) plus `IADD_IMM` of the member's byte offset in
  the block (`0xC` for `.a` and `0x10` for `.b`, after the three int
  words), then `LD_PKA.i32.unsigned`. The int members are
  combined with `ISUB.s32`/`IADD.s32` before `S32_TO_F32`, and the floats
  are flushed with `FMA.f32 x, 0, x`. The compiler's uniform array
  run-time index takes a float array's element only.

## 6. For the lowering (`lower-vsunif`)

Ranked by cases. Each one moves its reference from `REFUSED` to `MATCHED`
in `tests/mali_deqp_vsunif.rs` and removes its `#[ignore]` in the same
commit:

1. `random_dead_int` (111): dead-code elimination of int uniform reads
   before the type check.
2. `vec_subscript` + `tmp_array` + `matrix_subscript` + the dynamic part of
   `struct_local` (27 + 24 + 15 + 7): Mesa's bcsel tree for a run-time
   index of a local value, with the int word as a raw FAU compare operand.
3. `struct_local` (19) and `struct_int` (2): an int uniform carried through
   a local or an int add into `S32_TO_F32`.
4. `varying_array` (16): the masked `CSEL` writes of a dynamically indexed
   varying array.
5. `uapi_struct_int` (79, covered by `deqp_unif2_vs_struct_uniform`) and
   `struct_cmp` (2): int member compares (`ICMP_OR.u32.eq.m1`) in an AND
   chain.
6. `struct_dynidx` (2): `LD_PKA` of a struct array at a run-time index.

The covered shapes (`loop_bound_product` 44, `uapi_int` 15, the two loop
forms 12) are lowered with their own references and tests.
