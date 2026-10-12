# ES2 host dEQP-GLES2 at the integration tip, PowerVR and GC7000 (item pi4-es2-r1b2)

Every number below is **HOST-ONLY**. The library is built with the Linux OSAL, which keeps the
command stream instead of executing it, so every `glReadPixels` reads black. Only a case's
compile/link outcome, its info log and API error behaviour are real. Render verdicts are
meaningless and are not quoted as results; where a Fail count appears it is counted only to
show that it is a render verdict.

Measurement only, no source change. The V3D back end was not run.

## Identification

- Commit: `db3545fa25e23e27e09ef6127e34709e7d6a76e1` (the integration tip; the script labels
  its run directories `db3545f`). Tree clean, no source differs.
- VK-GL-CTS: the farm dependency cache checkout (environment name `CTS`), with
  `targets/yakogl/yakogl.cmake` present. `deqp-gles2` was built by the script itself.
- Mustpass list `tools/cts/gles2-mustpass.txt`, default chunking (100 cases per process).
- Library features, set by the script: `YAKOGL_OSAL=linux`, `YAKOGL_SHADERC=online`,
  `YAKOGL_REPORT_ES2=ON`, `YAKOGL_SHARED=ON`, `YAKOGL_GPU=powervr|vivante`.

## Commands

```sh
# environment names: CTS (VK-GL-CTS checkout), GPU (powervr | vivante; vivante is the default)
CTS=<VK-GL-CTS> GPU=powervr tools/cts/host-deqp-run.sh   # -> build/host-deqp/run-db3545f-powervr
CTS=<VK-GL-CTS> bash tools/cts/host-deqp-run.sh          # GPU=vivante -> build/host-deqp/run-db3545f
# each run ends by running: python3 tools/cts/cts-refusals.py --cases 2 <run directory>
python3 tools/cts/cts-refusals.py --cases 60 --tokens 3 build/host-deqp/run-db3545f
# scratch tallies (not committed): status per case and per group, read from the .qpa files
```

The whole mustpass list was run for both back ends.

## Counts, HOST-ONLY

| | GC7000 (`GPU=vivante`) | PowerVR (`GPU=powervr`) |
|---|---|---|
| Cases asked | 17165 | 17165 |
| Cases begun | 17165 (no shortfall) | 17165 (no shortfall) |
| Chunks | 172: 21 exit 0, 151 exit 1, 0 exit 134 | 172: 21 exit 0, 151 exit 1, 0 exit 134 |
| Compile refusals | 878 in 91 reasons | 878 in 91 reasons |
| Refusals on cases dEQP failed (gaps) | **0** | **0** |
| Refusals on NotSupported cases | 0 | 0 |
| Correct refusals (negative cases dEQP passed) | 878 | 878 |
| `functional.negative_api` | 237 Pass, 9 NotSupported, **0 Fail** (246) | the same |

`exit=1` is dEQP's status when any case of a chunk failed; with black read-back that is
expected and is not a crash. The per-case status tallies of the two back ends are
byte-identical (same cases, same statuses), and so are the failure-message tallies.

## Host-real Fail cases

**None.** The host-real groups were read from the `.qpa` files:

| Group (HOST-ONLY, both back ends) | Result | Real on the host? |
|---|---|---|
| refusals on failed cases (cts-refusals.py) | 0 | yes: compile/link |
| `functional.negative_api` | 237 Pass / 9 NotSupported / 0 Fail | yes: API errors |
| `functional.shader_api` | 38 Pass | yes |
| `functional.state_query` | 396 Pass | yes |
| `functional.fbo.completeness` | 370 Pass, 0 Fail | yes: completeness is a status query |
| `functional.uniform_api.info_query` | 159 Pass | yes |
| `functional.uniform_api.value` (no render) | 615 Pass | yes |

The 9 NotSupported are `negative_api.texture.compressedtexsubimage2d_*` (no compressed
formats are reported), as in the baseline.

Every other Fail is a render verdict that cannot be read here. The failure messages, over all
Fail results of one run (13,224 of them), are image comparison and pixel
checks: "Image comparison failed" 5654, "Fail" 5166 (shader cases, all of whose shaders
compiled and linked OK), "Image compare failed" 1017, "Image comparison failed." 616,
"Failed" 428, "Image verification failed" 260, "Got unexpected values" 40, "Buffer
verification failed" 37, "Got faulty pixels" 5, "Found invalid pixels" 1; plus 3
CompatibilityWarning (`flush_finish`). Those cases compile OK, so the only host evidence is
the compile; the verdict itself belongs to the board runs.

Render-failing groups, for the map of which item owns each (none is a host-real Fail):

| Group | Fail / asked-with-verdict (HOST-ONLY) | Why not host-real | Covered by |
|---|---|---|---|
| `shaders.*` (all subgroups) | 9104 Fail, 961 Pass, 36 NotSupported | render compare | `es2-preprocessor` (preprocessor 260 Fail, 256 Pass), `es2-fragdata-guard` (fragdata: `valid_*` three Fail, the invalid case Pass), `es2-invariance` / `es2-invariance-repro` (invariance), `es2-offline-table` |
| `uniform_api.value.*.render`, `uniform_api.random` | 393 + 35 Fail | render compare (the same group's non-render 615 + 159 Pass) | board rows; no host item needed |
| `fbo.render.*` and the rest of `fbo` | 319 Fail, 390 Pass (completeness 370 of the Pass) | render compare | `es2-v3d-fbo-combos-diag`, `es2-fbo-combos-test` |
| `fragment_ops` | 1880 Fail, 43 Pass | render compare | `es2-v3d-clear`, `es2-v3d-clear-jobs` |
| `color_clear`, `depth_stencil_clear`, `dither`, `depth_range`, `read_pixels`, `polygon_offset`, `rasterization` | 19, 11, 20, 28, 8, 5, 1 Fail | render compare | `es2-v3d-clear`, `es2-v3d-clear-jobs` |
| `texture.*` | 697 Fail, 110 NotSupported | render compare (`texture.completeness` 40 Fail are sampled-then-compared) | `es2-npot-cases`, `pi4-es2-r1a2` |
| `buffer`, `vertex_arrays`, `draw`, `default_vertex_attrib` | 47, 527, 89, 40 Fail | render or read-back compare | `es2-error-rules` (vertex/VAO tranche) covers the API half; the verdicts are board |
| `prerequisite.read_pixels` | 1 Fail | reads back black on the host | none needed |

Uncovered: **none** among host-real Fail cases, because there are none. The preprocessor
accept-but-wrong cases (`define_value_and_function`, macro expansion order) compile OK here
and so remain visible only on a board; that is the baseline's flag and it is unchanged.

## Differences from the baseline

Compared with `tools/cts/log/investigations/es2-host-deqp-baseline.md` (measured at
`1ad4695d`, 229 commits earlier; `es2-host-deqp-v3d.md` is the V3D record):

| | Baseline | Tip |
|---|---|---|
| Cases asked / begun, each back end | 17165 / 17165 | 17165 / 17165 |
| Chunks | 21 exit 0, 151 exit 1, 0 aborted | the same |
| Compile refusals | 878 in 92 reasons | 878 in **91** reasons |
| Gaps (refusals on failed cases) | 0 | 0 |
| Correct refusals | 878 on 876 distinct cases | 878 (the distinct-case count was not recomputed) |
| `negative_api` | 237 Pass / 9 NotSupported / 0 Fail | the same |
| GC7000 against PowerVR | identical buckets | identical buckets and identical per-case statuses |

The one difference is the reason count, 92 against 91, with the same 878 refusals; the top
reasons are unchanged (no implicit conversion in GLSL ES 1.00 264, expected a name 57, not
enough arguments for this constructor 54, expected a type 36, reserved for future use 36,
undefined identifier in a preprocessor expression 26, reserved in GLSL ES 1.00 24, extra
tokens at the end of the directive 22, undeclared identifier 21 (baseline 17), #if without
#endif 20, macro redefined with a different definition 20). I did not diff the full reason
lists, so which reason merged or went away is not identified. No refusal appeared on a case
dEQP failed, so the difference does not open a gap.

## What this does not show

The run cannot see accepted-but-wrong output (preprocessor expansion), invalid shaders we
accept, and every back-end table or render effect. The 246-case `negative_api` result and
the zero refusals on failed cases mean: at this tip, on the host, the PowerVR and GC7000
online compilers refuse no valid dEQP-GLES2 shader, and no `negative_api` case fails. It is
not a statement about conformance; the board trees are the arbiter.
