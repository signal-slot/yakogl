<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR online selector: a colour output narrower than a vec4 from an ALU result

Plan #338, item `pvr-int-out-note`. The fix is commit `89c06d85` (integration
branch; the same change is `3efd7b30` in the history this note was written
from, message "pvr online: a colour output narrower than a vec4 from an ALU
result builds"). Facts come from that commit, `src/shaderc/online/powervr/select.rs`
and the tests named below. The dEQP numbers are HOST-ONLY.

## 1. The shapes that failed

The online PowerVR selector refused the program with
`Usc(Select(Unsupported("a register read that nothing writes")))` when a
fragment colour output **narrower than a vec4** took its value from an ALU
result:

- `out int` / `out uint`: `o_out = v_in0 + 1;`, `o_out = v_in0 * 3u;`,
  `o_out = int(f);`, `o_out = uint(f);` (dEQP-GLES3
  `shaders.precision.{int,uint}.*_fragment`, row 5 `int-rt-pvr` of
  `docs/es3/shader-host-repro.md`);
- `out float`, `out vec2`, `out vec3` computed by arithmetic (same refusal).

What built: a copy of an input (a varying), a constant, a uniform.
`out vec4` from an ALU result built too, since the result has four lanes.

## 2. Root cause

`Sel` selection of `StoreOutput` for a `FragColor` slot took **four lanes**
for every colour output (`output_components`) and called `self.src(value,
lane)` for each. An ALU result writes only `result_lanes` registers: a scalar
writes lane 0. Lanes 1..3 therefore read `reg(value, 1..3)`, which nothing
writes. `eliminate_dead` checks that every register a group reads is written
or live-in, and refused the program there (the check near `select.rs:4632`).

Why the copy hid it: a program whose value is a varying copy has no ALU
groups, and a program with no groups skips the live-in check, so the same
unwritten reads went through. A constant and a uniform are materialised into
all lanes by `src`. So the defect looked like "integer outputs from ALU
results" while it was "output width against value width". Commit `5793f172`
(integer render-target packing) did not cause it and did not change it.

## 3. The invariant the code relies on now

`StoreOutput` for `IoSlot::FragColor(loc)` reads the **declared width** of the
output (`colour_components(loc)`, 1..4; 4 for `gl_FragColor`, `gl_FragData`
and an output the module does not list) from its value, and gives every lane
past that width **lane 0's source** (`select.rs`, the block after the `take(n)`
loop in the `StoreOutput` arm; `colour_components` next to `alu`).

Why repeat lane 0 and not leave the lanes out: GL leaves the undeclared
components undefined, but two later consumers need four defined registers:
the chain's output tail writes four registers per colour output, and the
discard epilogue reads the alpha lane. The last-value-per-slot rule (one
entry per slot, each output dword written once) is unchanged.

Anything that adds a colour-output path (a new render-target variant, a wider
integer packing) must keep reading only the lanes the value has.

## 4. The tests that pin it

- `tests/es3_int_out_alu_pvr.rs` (new in the commit): the USC bytes of the
  R32I/R32UI variant (`shc_blend_key_int_rt`) run through
  `usc_exec::run_binary`. `v + 1`, `v * 3u`, `int(v)`, `uint(v)` put the
  expected value in pixel output 0 (`int_add_lands_in_the_output`,
  `uint_arithmetic_lands_in_the_output`, `conversion_from_float_lands_in_the_output`,
  `copy_still_lands_in_the_output`), and `narrow_float_outputs_from_arithmetic_build`
  builds float, vec2, vec3 outputs from a sum. Without the fix 4 of the 5 fail
  with the refusal.
- `tests/es3_shaderc_repro.rs` `int_output_from_arithmetic_is_the_powervr_shape`:
  was a front-end-only check, is a build check now: int and uint, copy,
  constant, uniform, `v_in0 + 1`, `int(f)`, `uint(f)` build on `gpu-powervr`,
  `gpu-vivante` and `gpu-v3d`.
- `select.rs` unit test around line 5944 still pins the refusal text itself
  for a genuinely unwritten register.

Run the first with
`cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_int_out_alu_pvr`.

## 5. Host dEQP (HOST-ONLY)

`GPU=powervr MODULE=gles3`, VK-GL-CTS `93bca01`, `tools/cts/host-deqp-run.sh`:

| filter | before (`06d60e45`) | after (`c8770adc`) |
|---|---|---|
| `shaders.precision.int.*` | 24 began, 24 Fail, 12 compile refusals | 24 began, 24 Fail, 0 refusals |
| `shaders.precision.uint.*` | 24 began, 24 Fail, 12 compile refusals | 24 began, 24 Fail, 0 refusals |

The refusals are the 12 fragment cases per filter ("a register read that
nothing writes"). Fail stays at 24 per filter because the host back end's
read-back is black by design, so every case fails whether or not it builds:
these numbers show the compile fix, not a rendered value. The value is checked
only by the USC byte model above (no GPU arithmetic). No host row renders and
reads back an integer target, so a pass needs a board session.
