<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the corpus differential, our words against Mesa's per program, and its first table at `92b2601d5`

**HOST-ONLY.** Issue #11252, the first item of #11251
(`mali-compiler-road-to-es2.md` section 4.4 item 1). The tool is
`tools/mali/corpus_diff.py`; its header is the manual. This note says what
it measures and gives the first whole-corpus counts. Run tables are not
committed: run the commands below to get them.

## 1. What it measures

Every program of the corpus goes through our compiler and through Mesa
26.2.1, and gets one verdict:

| Verdict | Meaning |
|---|---|
| `equal` | ours compiles it to Mesa's words and facts in every region (words, work registers, preload, FAU words, secondary offset) |
| `differ` | ours compiles it to other words or facts |
| `refused` | ours refuses it (the detail is the `MaliError` text, or the log) |
| `noref` | ours compiles it and Mesa gave no reference |
| `invalid` | both refuse it, ours before the back end (a dEQP negative case) |
| `error` | the compare could not run (a panic, or a process that died) |

- **The unit is a program**, not a single shader. Mesa compiles programs,
  and a vertex shader's words depend on what its fragment shader reads.
  The detail of a `differ` row names the region (`vs.position`,
  `vs.varying`, `fs.fragment`), so the stage that differs is in the table.
- **Ours**: `tests/mali_corpus_diff.rs`, which builds each program as
  glCompileShader and glLinkProgram do on the Mali
  (`mali_ref::compile_pair`) and compares it with `tests/mali_ref`, the
  compare of every `deqp_*` test and of `mali-deqp-compiled-audit.md`.
  `tools/mali/route.sh`'s compare was tried first: it reads every uniform
  as a float and knows no struct member, so 16 of the 273 `tools/shaders`
  programs of the first try differed there by their FAU words alone. With
  `tests/mali_ref` they are equal. The other 18 differ under both, and no
  program route.sh found equal differs under `tests/mali_ref`.
- **Mesa**: `tools/mali/deqp_ref.sh --instrument shim` per program (the
  drm-shim draw with `--uniform-values`, the audit's method). When
  gl_ref.py cannot set a uniform (a `uvec2`, a struct with a sampler) the
  draw is made again with `--zero-uniforms`, and the compare then reads
  every uniform word as 0. References do not depend on our commit, so they
  are cached under `build/mali-corpus-ref/<digest of the reference tools>/`
  and the next commit makes only the new programs' references.

## 2. The corpus

- **dEQP-GLES2**: every program of every case of
  `tools/cts/gles2-mustpass.txt` that logs both sources.
  `tools/mali/deqp-host-measure.sh --cases` runs the whole list (not only
  the cases the board refused) and writes `programs.tsv` and `programs/`.
  A program's group is the dEQP group (`shaders.operator`) of most of the
  cases that use it; its uses are those cases.
- **glmark2**: the corpus of `tools/hostharness/glmark2/run.sh` (GPL-3.0
  text, never committed), paired by `tests/mali_glmark2_corpus.rs`'s
  `PROGRAMS` table. Uses: the default-list entries.
- **tools/shaders**: single shaders, paired per directory. `<stem>.vert`
  goes with `<stem>.frag` of the same GLSL version. A fragment shader
  without one goes with the directory's only vertex shader (femtovg's
  `main.vert`). Any other shader gets a generated partner that writes, or
  reads, each of its varyings from its own attribute (so Mesa cannot fold
  two varyings into one or move a constant across the stages). The 18
  desktop GLSL sources (`#version 450`) are skipped. A stem pair that is not
  a real program (`ret_nested`, `loop_nested`: the vertex shader does not
  write the fragment shader's varying) is reported, not hidden: Mesa's
  link refuses it.

## 3. The first table

- Compiler: origin/main `92b2601d5` (the tool was run from a branch on
  it that changes nothing under `src/`).
- Mesa 26.2.1 drm-shim, built by `tools/mali/drm_shim_ref.sh build` from
  the tarball of `third_party/mesa-26.2.1/UPSTREAM` (sha256 checked).
  Checked first: its reference of `tri_varying` has the committed
  reference's regions, byte for byte.
- VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c` with the yakogl
  target; glmark2 `22c527cb`.
- Commands (12 jobs on a shared 64-core host):

```sh
tools/mali/deqp-host-measure.sh --cts "$DEQP_WORK" --out "$PWD/build/measure" \
    --jobs 12 --chunk 100 --cases tools/cts/gles2-mustpass.txt
tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-host powervr   # GM2_WRITE=0 GM2_JOBS=12
MESA_SHIM_WORK=<shim work dir> python3 -I tools/mali/corpus_diff.py build/corpus-diff \
    --deqp build/measure --glmark2 build/glmark2-host/shaders --shaders tools/shaders --jobs 12
```

- **Time** (builds of deqp-gles2, the library, the shim and the glmark2
  harness not counted; they are one-off): the dEQP run itself 25 s for
  all 17,165 cases (all began; 167 processes exited 1, dEQP's "not every
  case passed", 5 exited 0, none crashed); the differential 3 min 43 s
  with a cold reference cache (217 s of it Mesa's 10,539 draws, 4 s our
  compiles) and 9 s with a warm one. Two runs gave the same `table.tsv`
  bytes.
- 12,447 of the 17,165 dEQP cases log at least one program with both
  sources: 10,641 distinct programs. All corpora: 10,960 programs, no text
  pair in two corpora.

| Corpus | Programs | equal | differ | refused | noref | invalid | error | Uses | equal (uses) | differ (uses) | refused (uses) |
|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| dEQP-GLES2 | 10,641 | 2,738 | 1,991 | 5,027 | 13 | 872 | 0 | 12,452 cases | 3,564 | 2,112 | 5,870 |
| glmark2 | 48 | 18 | 6 | 24 | 0 | 0 | 0 | 55 entries | 21 | 8 | 26 |
| tools/shaders | 271 | 202 | 18 | 51 | 0 | 0 | 0 | 271 | 202 | 18 | 51 |
| **all** | **10,960** | **2,958** | **2,015** | **5,102** | 13 | 872 | 0 | | | | |

Of the 4,729 dEQP programs ours compiles and Mesa draws, 1,991 (42 %) are
not Mesa's words. The audit estimated 33 % of the once-refused cases that
compile; this count takes every compiled program, not a sample.

The dEQP groups with the most programs:

| Group | Programs | equal | differ | refused | invalid |
|---|--:|--:|--:|--:|--:|
| shaders.operator | 2,949 | 928 | 853 | 1,168 | 0 |
| shaders.random | 1,819 | 228 | 102 | 1,489 | 0 |
| shaders.matrix | 738 | 176 | 241 | 321 | 0 |
| shaders.swizzles | 648 | 240 | 174 | 234 | 0 |
| shaders.loops | 613 | 159 | 120 | 334 | 0 |
| shaders.indexing | 568 | 45 | 89 | 434 | 0 |
| shaders.conversions | 524 | 131 | 170 | 169 | 54 |
| shaders.preprocessor | 512 | 254 | 4 | 4 | 250 |
| shaders.constant_expressions | 462 | 268 | 43 | 151 | 0 |
| uniform_api.value | 308 | 18 | 43 | 239 | 0 (8 noref) |

Where the 2,015 differing programs first differ (a region whose words
differ ranks before a fact): `vs.varying` words 1,382; `fs.fragment`
words 561; `vs.position` words 30; `fs.fragment` FAU words only 22; a
varying program Mesa does not have 12; `vs.position` FAU words only 8.
The vertex varying program leads, as the audit's V1 (vertex mediump and
lowp arithmetic kept at fp32 where Mesa narrows it to fp16) predicts.

The refusal texts with the most programs (all corpora): arithmetic other
than +, -, * and a dot or matrix product 1,438; a gl_FragColor lane other
than the varying's own 582; a uniform other than float, vecN or mat4 400;
a constant product Mesa's rules fold 367; a constant operation on a value
that is not a finite normal float 257; an integer or boolean value 213;
the back end defect `IrIndex` 206; a loop in a vertex shader 190. The
whole ranked list is in `summary.md` of the run.

## 4. Caveats

- The draw is gl_ref.py's, not dEQP's: attributes bound in declaration
  order, gl_ref.py's uniform values, one 64x64 triangle draw. A program
  whose words depend on the application's state differs for that reason:
  the 43 `attribute_location.*` programs bind locations dEQP chooses (25
  of them differ, 4 noref), and `builtin_variable.max_*_uniform_vectors`
  read a limit that is ours in our words and Mesa's in Mesa's.
- `noref` (13): 9 programs with a struct of samplers that gl_ref.py
  cannot bind (`GL_INVALID_OPERATION` at the draw), and the 4
  `attribute_location.bind_aliasing.max_inactive_*` programs, whose more
  than 16 declared attributes gl_ref.py's declaration-order binding cannot
  enable (`GL_INVALID_VALUE`).
- `invalid` (872) are programs both compilers reject: 256
  `invalid_implicit_conversions`, 250 `preprocessor`, 95 `keywords`, and
  the negative cases of other groups. A refusal of our back end with no
  Mesa reference is still `refused` (32 rows), since the front end took
  the program.
- One `tools/shaders` program (`es100/dynvec4.frag` with its partner)
  aborts Mesa's debug build with `--uniform-values` (its IR validator, the
  text in the draw's stdout); ours refuses it.
- `tests/mali_deqp_audit.rs` (`differs_pinned`: `l1_matrix_sub_uniform_mediump_mat3_vs`
  is now byte-equal) and `tests/mali_deqp_idxloc.rs` (`refusals_pinned`:
  `fs_mat2_dd`'s refusal moved) fail at `92b2601d5` already, without this
  change; neither file is a row of `tools/check.sh`.
