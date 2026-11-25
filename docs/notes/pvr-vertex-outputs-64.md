# The src/gl change that lets the PowerVR answer GL_MAX_VERTEX_OUTPUT_COMPONENTS 64

Plan #400, item vout-gl-design. This note names the change; it is not applied
here, because `src/gl/**` belongs to plan #397. A later plan applies it.

## 1. The defect

The AM62P (PowerVR) answers `GL_MAX_VERTEX_OUTPUT_COMPONENTS` 60. ES 3.0
(table 6.31) asks at least 64, so
`dEQP-GLES3.functional.implementation_limits.max_vertex_output_components`
fails with "Requirement not satisfied". The 60 is a software choice:
`src/gl/context.rs:262` answers `MAX_VARYING_VECTORS * 4` for every GPU except
the V3D, whose arm (`:258-260`) answers 64.

## 2. Why 64 is safe on the PowerVR

`tests/pvr_vertex_outputs_64.rs` (merged, item pvr-vout64-test) pins two facts
on the host:

- `sixteen_vec4_outputs_with_builtins_compile_and_link`: a vertex shader with 16
  `vec4` outputs plus `gl_Position` and `gl_PointSize`, read in full by the
  fragment shader, compiles and links on the PowerVR path. That is 64
  components plus the builtins, which is what a program at the new limit declares.
- `seventeen_vec4_outputs_do_not_link`: the real ceiling is the 16 varying rows
  of a `ShcShader` record (`src/shaderc/mod.rs`), refused with "more
  varyings than the record holds" (`assemble.rs` `vertex_output_writes`). 64
  components is exactly 16 rows, so the reported limit then equals the
  enforced one and no program the limit permits is refused.

No GL-core check counts vertex outputs against the reported limit; it is only
reported. The fragment side stays 60 (`MAX_FRAGMENT_INPUT_COMPONENTS`,
`MAX_VARYING_COMPONENTS`, `MAX_VARYING_VECTORS` 15): ES 3.0 asks 60 there and the
PowerVR record carries 15 varyings to the fragment stage.

## 3. The patch

Make the V3D arm the general ES 3.0 arm. The V3D reason (the VPM record is
sized by what the fragment shader reads) is kept in the comment, and the
PowerVR reason is added. Under `report-es2` nothing changes (ES 2.0 does not
report the query; `tests/es2_report_guard.rs` lists the enum as refused).

```diff
--- a/src/gl/context.rs
+++ b/src/gl/context.rs
@@ -255,10 +255,15 @@
 /// the components the linked fragment shader asks for), so a vertex shader
 /// may declare 16 vec4 outputs as long as 15 are read.
-#[cfg(all(feature = "gpu-v3d", not(feature = "report-es2")))]
+///
+/// PowerVR: 64 as well: a `ShcShader` record holds 16 varying rows, and 16
+/// vec4 outputs with gl_Position and gl_PointSize compile and link
+/// (tests/pvr_vertex_outputs_64.rs); the 17th row is refused there.
+#[cfg(all(any(feature = "gpu-v3d", feature = "gpu-powervr"), not(feature = "report-es2")))]
 pub const MAX_VERTEX_OUTPUT_COMPONENTS: usize = 64;
-#[cfg(not(all(feature = "gpu-v3d", not(feature = "report-es2"))))]
+#[cfg(not(all(any(feature = "gpu-v3d", feature = "gpu-powervr"), not(feature = "report-es2"))))]
 pub const MAX_VERTEX_OUTPUT_COMPONENTS: usize = MAX_VARYING_VECTORS * 4;
--- a/src/gl/state.rs
+++ b/src/gl/state.rs
@@ -3243,11 +3243,11 @@
-         * (60 of the 64 fragment inputs the record carries) and 64 vertex
-         * output components; Mali: 8, ES 2.0's minimum, until more are
-         * measured on the board */
+         * (60 of the 64 fragment inputs the record carries) and 64 vertex
+         * output components; PowerVR: 64 vertex output components too (16
+         * rows of the record); Mali: 8, ES 2.0's minimum, until more are
+         * measured on the board */
@@ -3251,1 +3251,1 @@
-        let outputs = if cfg!(feature = "gpu-v3d") { 64 } else { vectors * 4 };
+        let outputs = if cfg!(any(feature = "gpu-v3d", feature = "gpu-powervr")) { 64 } else { vectors * 4 };
```

(Line numbers are those of the tip at bdd2ba1c; the hunks are written by
content, so apply by context. `state.rs:3251` is the `let outputs` line.)

A test file the patch must also change, or it fails: `tests/es3_pvr_varyings.rs`,
`the_four_limits_read_fifteen_vectors_and_sixty_components` (line 419) asserts
60 for `GL_MAX_VERTEX_OUTPUT_COMPONENTS`; it becomes 64 with the message
"GL_MAX_VERTEX_OUTPUT_COMPONENTS: 16 * 4, ES 3.0 asks 64" (and the test name
and the other three asserts stay: they are the fragment side, still 60).

## 4. What changes elsewhere

- `GL_MAX_VERTEX_OUTPUT_COMPONENTS` is also what the shader's
  `gl_MaxVertexOutputVectors` reads (`src/shaderc/online/reflect.rs:1287`,
  `lower.rs:14242`, as `/ 4`): it goes from 15 to 16 on the PowerVR. Nothing in
  the tree pins 15 for it (grep of `tests/`: none).
- Vivante: the second arm still applies (`MAX_VARYING_VECTORS * 4`), no change.
  V3D: no change (already 64). Mali: no change (8 * 4). `report-es2`: no change.
- Transform feedback (`src/gl/tf.rs:41`) quotes the limit in a comment only.
- dEQP cases that move on the AM62P: `implementation_limits.max_vertex_output_components`
  (Fail to Pass, am62p-gles3-notpass.tsv line 762), and the
  `state_query.integers.max_vertex_output_components_*` rows (getfloat,
  getinteger64: "Got invalid ... value", TSV lines 1371-1372, and any sibling
  row) if they compare with 64. No case regresses: a shader that declares at
  most 16 rows already linked, and one with more is refused as before.

## 5. The host test that pins it

- `src/gl/state.rs` unit test `the_es3_limits_are_what_this_gpu_carries_and_programs`
  (the `outputs` line above) with
  `cargo test --no-default-features --features host-test,gpu-powervr --lib the_es3_limits`.
- `tests/es3_pvr_varyings.rs` (`cargo test --no-default-features --features
  host-test,gpu-powervr,shaderc-online --test es3_pvr_varyings`), changed to 64.
- `tests/pvr_vertex_outputs_64.rs`, unchanged, keeps the 16-rows-link and
  17th-row-refused evidence under it.

Run also the Vivante and V3D feature sets of the state.rs test to see them
unchanged (`gpu-vivante`, `gpu-v3d`).

## 6. Board check

None needed beyond the AM62P dEQP run: the answer is a host number, and the
link behaviour is the PowerVR compiler's, covered on the host.
