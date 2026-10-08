# ES2 host dEQP-GLES2 baseline (item es2-host-deqp-baseline)

Every number below is **HOST-ONLY**: the library is built with the LINUX OSAL, which keeps
the command stream instead of executing it, so every `glReadPixels` reads black. Only each
case's compile/link outcome and info log are meaningful. No render verdict (Pass/Fail counts
over rendering cases) is quoted as a result.

Status: **GC7000 (vivante) and PowerVR measured.** V3D has no back-end switch in the script
and was not run.

## Identification

- Integration commit: `1ad4695d64c07cf1ed8ab0d4dba4d34f668a400d`. The script labels the run
  `49fb653`, which is that commit plus one docs-only commit (the earlier version of this file).
  No source file differs from the integration tip.
- VK-GL-CTS: `/home/tasuku/.local/share/ccfarm/deps/yakogl/VK-GL-CTS` at
  `93bca01861b0e3ef3c387027a9791e6d065f900c` (pinned in `docs/es3-host-env.md`), with
  `targets/yakogl/yakogl.cmake` present. `deqp-gles2` was built by the script into the
  workspace's `build/host-deqp/deqp`.
- Tools unmodified. Default chunking (100 cases per process). Mustpass list
  `tools/cts/gles2-mustpass.txt`.

## Commands

```sh
CTS=/home/tasuku/.local/share/ccfarm/deps/yakogl/VK-GL-CTS GPU=vivante tools/cts/host-deqp-run.sh
#   -> build/host-deqp/run-49fb653 ; it ends by running tools/cts/cts-refusals.py --cases 2 on it
python3 tools/cts/cts-refusals.py --cases 60 --tokens 3 build/host-deqp/run-49fb653
# scratch (not committed): group/negative_api split over the same .qpa files, reusing
# scan() from cts-refusals.py (build/scratch/groups.py)
CTS=/home/tasuku/.local/share/ccfarm/deps/yakogl/VK-GL-CTS GPU=powervr tools/cts/host-deqp-run.sh
#   -> build/host-deqp/run-01fb562-powervr
```

## GC7000 back end (GPU=vivante), HOST-ONLY

| | |
|---|---|
| Cases asked | 17165 |
| Cases begun | 17165 (no shortfall) |
| Chunks | 172: 21 exit 0, 151 exit 1, **0 aborted (no exit 134)**, no panics |

`exit=1` is dEQP's status when any case in the chunk failed; with black read-back that is
expected and says nothing about the compiler. It is not a crash.

### cts-refusals.py

- 172 .qpa files, 878 compile refusals in 92 distinct reasons.
- **Gaps (refusals on cases dEQP failed): 0** in 0 reasons. NotSupported (excused): 0.
- **Correct refusals (negative cases dEQP passed): 878** refusals on **876 distinct cases**,
  all in `functional.shaders` (no other dEQP group has a compile refusal).
- Top reasons, all correct refusals: no implicit conversion in GLSL ES 1.00 (264); expected a
  name (57); not enough arguments for this constructor (54); expected a type (36); reserved for
  future use (36); undefined identifier in a preprocessor expression (26); reserved in GLSL ES
  1.00 (24); extra tokens at the end of the directive (22); #if without #endif (20); macro
  redefined with a different definition (20); undeclared identifier (17); a condition must be
  a bool (16); and 80 more reasons.

Gap buckets by dEQP group: none, so there is no bucket with case names to list.

### What the zero means, and what it hides

The tool only counts a *compile refusal on a case dEQP failed*. It cannot see the other
kinds of ES2 failure, which the owner's stage 1 also has to close:

- shaders we **accept but expand or evaluate wrongly** (preprocessor `define_value_and_function`,
  macro expansion order): the compile succeeds, so there is no refusal to bucket, and the
  render verdict that would reveal it is meaningless on the host;
- invalid shaders we **accept** (the negative cases dEQP fails because we did not refuse);
  these do not produce a refusal either;
- invariance, scissor/mask clear, texture wrap/completeness, offline-table: render or
  back-end-table effects invisible to this run.

So "0 gaps" means: at the integration tip the GC7000 online compiler refuses no valid
dEQP-GLES2 shader, on the host. It is not a statement about conformance.

### negative_api error rules (API-level, no rendering involved), HOST-ONLY

`functional.negative_api.*`: 246 cases ran. 237 passed, 9 NotSupported, **0 failed**.
The 9 NotSupported are all `negative_api.texture.compressedtexsubimage2d_*`
(`invalid_size`, `invalid_target`, `level_max_cube`, `level_max_tex2d`, `neg_level_cube`,
`neg_level_tex2d`, `neg_offset`, `neg_wdt_hgt`, `offset_allowed`): the library reports
`GL_NUM_COMPRESSED_TEXTURE_FORMATS` = 0, so dEQP excuses them.

Cases where an ES 3-only enum or format is accepted: **none seen in this run** (0 failures
in the group). This build is `YAKOGL_REPORT_ES2=ON`. Caveat: dEQP's GLES2 negative_api
group covers only part of the ES 3-only surface; the static audit is the item
`es2-error-rules-diag` (`docs/investigations/es2-error-rules.md`), and the board runs are
the arbiter.

## PowerVR back end (GPU=powervr), HOST-ONLY

Run after the farm granted the command; label `01fb562` (the integration tip plus this
document's commits), same CTS, same default chunking, run directory
`build/host-deqp/run-01fb562-powervr`.

| | |
|---|---|
| Cases asked | 17165 |
| Cases begun | 17165 (no shortfall) |
| Chunks | 172: 21 exit 0, 151 exit 1, **0 aborted (no exit 134)** |

- cts-refusals.py: 878 compile refusals in 92 reasons; **gaps 0**; NotSupported 0;
  **correct refusals 878 on 876 distinct cases**, all in `functional.shaders`. The top
  reasons and counts are identical to the GC7000 run.
- negative_api: 246 cases, 237 passed, 9 NotSupported (the same nine
  `compressedtexsubimage2d_*` cases), 0 failed. No ES 3-only enum or format accepted.
- Both back ends give the same buckets. Neither run showed a back-end refusal (the
  `online shaderc: the code generator refused ...` form that cts-refusals.py also reads), so on
  the host neither the PowerVR nor the GC7000 code generator refuses any dEQP-GLES2 shader.
  I did not verify that the code-generator stage runs for every compiled shader.

## Notes on running

The first attempts to run the script were refused ("This command requires approval") until the
farm granted `tools/cts/host-deqp-run.sh`; the vivante and powervr runs then went through. No
tool defect was found. V3D has no `GPU=v3d` switch in the script as read at this commit, so no
V3D host number exists.

## Cross-reference: gap bucket to plan item

| Bucket (HOST-ONLY, GC7000 and PowerVR identical) | Count | Item that closes it |
|---|---|---|
| Compile refusals on failed cases | 0 | nothing to close |
| Correct refusals (negative GLSL, 876 cases) | 878 refusals | not work: implementing any of these reasons would turn passes into failures; `es2-preprocessor` must keep the 26 "undefined identifier in a preprocessor expression", 22 "extra tokens", 20 "#if without #endif", 20 "macro redefined" refusals |
| negative_api ES 3-only enum/format accepted | 0 failures | `es2-error-rules` (static audit in `es2-error-rules-diag`) |
| Preprocessor accept-but-wrong (define_value_and_function, expansion order) | not visible to this tool | `es2-preprocessor` (pp.rs) |
| shaders.invariance.common_subexpression_1/2/3 | not visible (board only, 9 AM62P failures) | `es2-invariance` |
| Offline-table / no-table path | not visible (GL suite, board) | `es2-offline-table` |
| Texture wrap/completeness (NPOT) | not visible | `es2-npot-cases` |
| Scissored/masked glClear on V3D | not visible, and V3D not a host back end | `es2-v3d-clear` |
| Pi 4 fbo.completeness `*_tex` | not visible | `es2-v3d-fbo-combos-diag` (no fix item yet) |
| PowerVR host compile gaps | 0 | nothing to close |

Flagged as uncovered: any host check for
accepted-but-wrong preprocessor output and for accepted invalid shaders, which
`cts-refusals.py` cannot produce. A host test that diffs our preprocessed output against the
expected expansion (in `tests/es2_preprocessor.rs`) is the only way to see them without a board.
