# YakoGL host tools

`tools/yakogl-tools` is one Rust program (a workspace member; the library build
never compiles it or its dependencies) with a subcommand per generator:

| Command | Writes |
|---|---|
| `yakogl-tools glsl2usc [options] inputs...` | the offline shader table `tests/reference/blobs.rs` |
| `yakogl-tools usc-blob <src> <file> <SYMBOL> [VARIANT]` | one internal shader's block in `src/pvr/blobs.rs` |
| `yakogl-tools femtovg-shaders [<femtovg dir> [outdir]]` | the FemtoVG 0.26 sources in `tools/shaders/femtovg/` |
| `yakogl-tools gl-consts` | `src/gl/consts.rs`, `src/gl/egl_consts.rs` |
| `yakogl-tools abi-layout <driver src>` | `src/abi_layout.rs`, `src/abi_layout_check.c` |
| `yakogl-tools shader-check [--codegen]` | nothing; answers JSON compile/link requests on stdin |

`tools/precompile` is a second, separate program, `yakogl-precompile`: the source
list compiled with our own compiler ("The source list and the precompiler" below).

Run it with `cargo run -p yakogl-tools -- <command> ...` (or build it once with
`cargo build -p yakogl-tools` and call `target/debug/yakogl-tools`); its tests are
`cargo test -p yakogl-tools`. The tools find their default inputs and outputs
relative to the repository they were built from (`YAKOGL_TOOLS_ROOT` points them at
another checkout).

They replaced Python scripts (`glsl2usc.py`, `gen_usc_header.py`,
`gen_femtovg_shaders.py`, which upstream C still carries at `a3b7374`, and this port's
`gen_gl_consts.py` and `gen_abi_layout.py`) and behave identically: the same table
entries, text and messages (the port and how it was verified:
`tools/yakogl-tools/src/glsl.rs`).

# Compiling at run time (dEQP-GLES2)

The offline table can only hold shaders that existed at build time, and the
Khronos CTS writes its shaders as it runs. A build with the cargo feature
`shaderc-online` compiles them in the library itself (`src/shaderc/online/`,
`src/shaderc/SHADERC_NOTES.md`).

# The Mesa reference

`tests/reference/blobs.rs` (PowerVR, `tools/glsl2usc.sh`, pco_shc) and
`tests/reference/vivante_blobs.rs` (GC7000, `tools/vivante/gen_shader_table.py`,
etna_shc) are what Mesa's compilers made of the source list below. They are the
reference our compiler is measured against, compiled as `shaderc::mesa_reference`
into the host tests (`host-test`) and into one kind of product build only: a
measurement image that takes the reference as its shader table (cargo feature
`mesa-reference-table`, which Kconfig `YAKOGL_SHADERC_OFFLINE` and the C builds'
`SHADERC offline` select; the bare-metal build, which has no compiler yet). No
other build links a Mesa-built row: `tools/check.sh` builds the image archive for
every OSAL and GPU with the compiler and counts the reference's symbols ("no Mesa
reference in the artefact"). The precompiled table below will replace the
measurement use.

# The source list and the precompiler

`tools/shaders/precompiled.list` is the one list of the shaders compiled ahead of
time, for every table; its head documents the syntax. A line is one directive:

```text
dir es100 gc7000                      # a directory, and what it is for
src femtovg/fill_color.frag           # an input of the PowerVR table, in row order
blend one,inv_src_alpha               # a variant every fragment shader gets ...
for qt3d/*.frag one,inv_src_alpha;replace@f16x4   # ... unless a glob picks others
app femtovg                           # the programs an application links:
pair femtovg/main.vert femtovg/fill_color.frag
```

Three generators read it: `tools/glsl2usc.sh` (the PowerVR Mesa reference from
pco_shc: `src`, `blend`, `for`, `skip-failures`, through `yakogl-tools glsl2usc
--sources`, which regenerates the committed `tests/reference/blobs.rs` byte for
byte),
`tools/vivante/gen_shader_table.py` (the GC7000 table from etna_shc: its directory
allowlist is the `dir` lines with the `gc7000` role, and a directory with no line
fails it) and `yakogl-precompile`, which reads all of it. The list's Rust reader
(`tools/precompile/src/list.rs`) and the variant spec parser are compiled into both
Rust tools. The `gl_test` and `gl_test_es100` apps are what `gl_test.c` links,
read out of its text: `tools/precompile/gl_test_pairs.py` prints them and
`--check`s the list against them.

**`yakogl-precompile`** (`tools/precompile`, a workspace member that depends on
nothing but the library) compiles the list with our own compiler, on the host, into
the table an image carries in front of the compiler (the precompiled tier,
below). It links the library as an image does -- `shaderc-online`,
`no-shader-table`, and not `host-test`, which changes code the compiler runs -- and
drives `SHC_ONLINE`, the back end an image installs, through its own entry points
in the GL core's order: each listed program's sources compiled, linked (so the
link's own records are made: a fragment shader compiled again with a varying the
vertex shader never writes read as 0, or for a constant `gl_Position.w`; a vertex
shader laid out with the program's attribute locations), and the variants a draw
asks for asked of the linked fragment shader (the list's specs on the PowerVR; the
opposite winding of a `gl_FrontFacing` reader on the GC7000); each source no
program links compiled alone. Every record the compiler's cache held is a row.

```sh
cargo run -p yakogl-precompile --features gpu-powervr -- [-o OUT] [--app NAME]...
cargo run -p yakogl-precompile --features gpu-vivante,report-es2 -- ...
```

| Option | Default | |
|---|---|---|
| `-o OUT` | `build/precompiled/<gpu>[-es2].rs` | the table (Rust; never committed) |
| `--list LIST` | `tools/shaders/precompiled.list` | |
| `--app NAME` | every app and every listed source | one application's programs only (repeatable) |
| `--refusals FILE` | `tests/fixtures/online_refused_<gpu>[_es2].txt` | the sources the compiler refuses (`tests/online_corpus.rs` pins them message for message): left out with their pairs; a refusal not listed, or a listed source that compiles, fails the run |
| `--refused-steps FILE` | `tests/fixtures/precompile_refused_<gpu>[_es2].txt` | the links and variants it refuses (a program the GL will not link, a blend state whose draws it refuses): the whole list's run must refuse exactly these; `PRECOMPILE_REFUSED_WRITE=1` writes them |
| `--verify TABLE` | | TABLE must be the regeneration, and each of its rows what the compiler makes for that row alone in a fresh cache |
| `--pin COUNTS` | | the run's row and code-byte counts must be COUNTS' line (`tests/fixtures/precompile_counts.txt`; `PRECOMPILE_PIN_WRITE=1` writes it) |

The GPU is a feature, exactly one (`gpu-powervr`, `gpu-vivante`), and
`report-es2` builds the compiler of an image that reports ES 2.0. The table is the
`ShcBlob` table `emit.rs` writes, sharing its printers (`shc_print.rs`) and its
idiom (each row a library struct printed with a full destructuring pattern), with
the columns our rows need beside it: the record key the compiler's cache finds a
row by (the source's SHA-256 chained with the zeroed varyings, the attribute map
and the facts), the source as an index into `SHC_PRECOMPILED_SOURCES`, the facts
(`w_constant`, the sampler-state key, the attribute map) and the uniform blocks. Its
header carries the toolchain mark, which says which compiler made it:

```text
// toolchain: yakogl=<crate version> commit=<git sha>[+dirty] gpu=<powervr|gc7000> es2=<0|1>
```

The internal shaders (`internal/`: fbload, clear; GLSL 450 that the back ends carry
themselves) are out of its scope for now and skipped by count. `tools/check.sh`
lints and tests it per build, and `tools/precompile/prove.sh FEATURES` runs its
proofs: the table made twice is the same bytes, `--verify`, the counts of the whole
list and of the `femtovg` and `gl_test_es100` sublists pinned, and the table
compiled against the library (`rustc -D warnings`).

Beside the rows the table carries every link the generator made, by record key
(`SHC_PRECOMPILED_LINKS`: the verdict a link takes without the front end), every
row's sampler-state sensitivity (what a program's first draw asks), and the
toolchain mark as constants (`SHC_PRECOMPILED_GPU`, `_ES2`, `_TOOLCHAIN`).

## The precompiled tier

With the crate feature `precompiled-shaders` (Kconfig
`YAKOGL_PRECOMPILED_SHADERS`, CMake `YAKOGL_PRECOMPILED_SHADERS`; off by
default) the library includes a table this tool made and looks every shader up
in it before compiling (`src/shaderc/online/precompiled.rs`). The order, at each
of the compiler's entry points (compile, a blend variant, a link's substitution,
the vertex shader laid out at link, a sampler-state variant): the keep cache,
then the table, by the same record key and stage and blend key, then the
compiler. A hit runs no compile; a link the table recorded takes its verdict
from it and a row's sensitivity is read, not computed, so a program made only of
rows runs the compiler not once (`shc_precompiled_hits()` against
`shc_online_compiler_runs()`; on the host the gl_test_es100 app's 100 PowerVR
programs replay with 0 runs, against 3240 without the table). The table is
consulted only while the compiler's switches stand at their boot values, and
`shc_precompiled_set_enabled(0)` turns it off for a measurement. The image logs
the table's mark at the first look-up, and a table made for another GPU or ES
level does not build.

```sh
cargo run -p yakogl-precompile --features gpu-powervr -- --app gl_test_es100 -o build/precompiled/t.rs
YAKOGL_PRECOMPILED_TABLE=$PWD/build/precompiled/t.rs \
  cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,precompiled-shaders
```

An image build does the first line itself (capi/yakogl_capi.cmake, `PRECOMPILED`,
into its build directory; `YAKOGL_PRECOMPILED_APPS` picks the apps) on every
build; the table is rewritten only when it changed.

# Building the pco_shc that reproduces the table

`PCO_SHC` must point at a `pco_shc` built by `tools/pco_shc/build_pco_shc.sh`:
Mesa 26.2.1 from the release tarball, `tools/pco_shc/pco_shc.c` dropped in,
`tools/pco_shc/usclib-tex.cl.patch` applied **and the usclib compiled from it in
the same build** (the script builds Mesa's `mesa_clc` for that, so the host needs
clang/LLVM, libclc and SPIRV-LLVM-Translator; the package list is in the
script's header):

    tools/pco_shc/build_pco_shc.sh ~/mesa-26.2.1.tar.xz ~/mesa-build
    PCO_SHC=~/mesa-build/mesa-26.2.1/build/src/imagination/vulkan/pco_shc tools/glsl2usc.sh

The script checks the tarball against `third_party/mesa-26.2.1/UPSTREAM`, and
refuses a usclib whose `usclib_tex_state_base_level` does not test the
texture type against `ROGUE_TEXSTATE_TEXTYPE_STRIDE` (it disassembles
`pco_usclib.spv` with `spirv-dis`). It builds with
`-ffile-prefix-map=<tree>=/mesa`, so the binary does not depend on the
directory it was built in: two builds in two directories gave the same bytes.
It prints the path and the content id `yakogl-tools glsl2usc` will report.

That binary reproduces **every one of the 2248 rows** `tests/reference/blobs.rs`
held before it was regenerated with it, byte for byte (built 2026-10-05 with
LLVM/clang/libclc 23.1.2, SPIRV-LLVM-Translator 23.1.1, gcc 16.2.1, and
`glslangValidator` 16.4.0). The table now names that toolchain in its header:

    // toolchain: pco_shc=f537fc86b41f glslang=9e007f4bf514

and `glsl2usc` refuses to overwrite the table with a different one unless
`GLSL2USC_NEW_REFERENCE=1` says so. A different compiler or library version
may give a different id for the same output; check the rows with a comparison
before you accept a new reference.

**Never use a `pco_shc` whose usclib was not rebuilt from the patched
`tex.cl`.** History: the patch makes `textureSize()` and
`textureQueryLevels()` skip `baselevel` for a STRIDE (linear) image, whose
word-1 bits 60..63 are `num_mip_levels`; without it every size came out
halved that many times, and YakoGL samples every texture this way. A
development host once had the patched `tex.cl` but could not rebuild the
usclib (no LLVM/libclc for the build graph), so its `pco_shc` carried a stale
`pco_usclib.spv` (md5 `32d677f94ad9`, no `OpIEqual`/`OpBranchConditional` in
that function) and reproduced only 1735 of the 2042 rows the table held then:
the 307 others were exactly the shaders inlining a usclib texture routine
(`samplerCube`, `sampler2DArray`, `sampler3D`, `textureSize`, `texelFetch`),
10..62 dwords shorter. The committed table was verified on silicon (34/34
`gl_test` cases on the AM62P) with the patch; a table from such a binary would
silently bring the `textureSize()` bug back.

# glsl2usc — build-time GLSL-ES → PowerVR USC blob generator

`glsl2usc` compiles GLSL shaders to PowerVR **USC** machine code + the driver
wiring metadata, and emits the Rust table (`tests/reference/blobs.rs`) the *offline*
shader backend (`src/shaderc/offline.rs`) looks shaders up in by source hash. This is the
offline half of the shader strategy in `yakogl/DESIGN.md` (§ Shaders): on
Zephyr / bare-metal the PCO+glslang toolchain is too big to run on-target, so app
shaders are precompiled here and embedded.

## Pipeline

```
  *.vert / *.frag
        │  GLSL ES: glslangValidator -S <vert|frag>     (the ES rules, on THIS text)
        │  GLSL ES: glslangValidator -E -S <vert|frag>  (the ES preprocessor)
        │  (GLSL ES 1.00 / 3.00: the result rewritten into Vulkan GLSL 450)
        │  glslangValidator -V -S <vert|frag>       (GLSL → SPIR-V)
        ▼
      *.spv
        │  pco_shc <vs|fs> in.spv                   (SPIR-V → NIR → PCO → USC)
        ▼
  USC dwords (stdout) + metadata (stderr)
        │  parse metadata → ShcShader; sha256(source)
        ▼
  tests/reference/blobs.rs   (one entry per shader)
```

The first two steps are the GLSL ES front end (`tools/yakogl-tools/src/es100.rs`):
an ES source is judged by GLSL ES's own rules, on the bytes the application
wrote, before anything rewrites it, and it is the *preprocessed* text that gets
rewritten. Both are skipped for a GLSL 450 source (this project's internal
shaders), and neither changes the table key, which stays the SHA-256 of the
file's original bytes. `src/shaderc/SHADERC_NOTES.md` has the reasoning and the
measurement.

`pco_shc` is the harness built on the production Mesa PCO backend for the
BXS-4-64 (BVNC 36.53.104.796); it runs the same lowering the real PVR Vulkan
driver runs and copies the driver's register-allocation helpers verbatim, so the
vtxin / coefficient / shared-register layout is identical to shipping silicon.

Every table row is built as the library's own `ShcShader` / `ShcVarying` /
`ShcAttrib` / `ShcUniform` values (full struct literals) and printed from them, so
a field added to those structs fails the tool's build until the emitter handles it.

## Requirements

- `glslangValidator` on `PATH` (GLSL → SPIR-V).
- `pco_shc` built from the Mesa tree by `tools/pco_shc/build_pco_shc.sh` (see
  above; `src/imagination/vulkan/pco_shc.c` there is the project's own harness,
  not upstream). Default path points at the scratchpad
  build: `../scratchpad/mesa-build/mesa-26.2.1/build/src/imagination/vulkan/pco_shc`
  relative to the repository (set `PCO_SHC` otherwise).
- A Rust toolchain (the tool is built by cargo).

## Usage

Regenerate `blobs.rs` (and the fbload blocks of `src/pvr/blobs.rs`) from the
source list `tools/shaders/precompiled.list` (canonical, deterministic; the list
is below, "The source list and the precompiler"):

```sh
tools/glsl2usc.sh
GLSL2USC_CACHE=~/.cache/glsl2usc tools/glsl2usc.sh   # reruns take seconds
```

Or drive the tool directly:

```sh
cargo run -p yakogl-tools -- glsl2usc -o tests/reference/blobs.rs \
    tools/shaders/uniform_color.frag tools/shaders/passthrough.vert \
    tools/shaders/interp_color.frag tools/shaders/color_uv.vert \
    tools/shaders/color_uv.frag tools/shaders/mvp.vert tools/shaders/texture.frag
```

List each shader's stage and SHA-256 (after compiling it) without emitting the table:

```sh
cargo run -p yakogl-tools -- glsl2usc --list tools/shaders/*.vert tools/shaders/*.frag
```

Options: `-o FILE` (default: stdout), `--list`, `--blend SPEC` (repeatable, see
below), `--for GLOB=SPEC[;SPEC...]` (the variant list for the fragment shaders whose
path matches GLOB, instead of the `--blend` list; first match wins),
`--pco-debug GLOB=FLAGS` (pco_shc runs under `PCO_DEBUG=FLAGS` for every variant of
the shaders whose path matches GLOB, first match wins: `no_pred_cf` builds an `if`
under the execution mask with a branch over each arm instead of predicating it;
the setting is part of the cache key and each such row says so in the table, on
its header line and above its USC), `--skip-failures` (a shader the compiler cannot
build is reported and left out of the table instead of aborting), `--sources LIST`
(the inputs, `--blend`, `--for` and `--skip-failures` read from the list's `src`,
`blend`, `for` and `skip-failures` lines first; inputs are then optional). Long
options may be abbreviated.

### Config (env vars)

| Var | Default | Meaning |
|---|---|---|
| `PCO_SHC` | `../scratchpad/.../pco_shc` (relative to the repository) | pco_shc binary |
| `GLSLANG` | `glslangValidator` (PATH) | GLSL→SPIR-V compiler |
| `GLSL2USC_CACHE` | unset | directory caching the compiler's output, keyed by the compiled text, stage, variant, the pco_shc binary (size, mtime) and the `PCO_DEBUG` it runs under (`--pco-debug`'s or the environment's; only when set). Writes are not atomic: an interrupted run leaves zero-length entries the next run serves as shaders with no code — delete the directory after a kill |
| `GLSL2USC_KEEP` | unset | keep the temporary directories; if it names a directory, the rewritten GLSL (`.es100to450.glsl`) and the metadata (`.meta.txt`) are written there |
| `GLSL2USC_UNROLL` | unset | unroll `for` loops over uniform arrays (the old text; a different table key) |
| `YAKOGL_TOOLS` | unset | `glsl2usc.sh`: a prebuilt `yakogl-tools` instead of `cargo run` |
| `GLSL2USC_OUT` | unset | `glsl2usc.sh`: write the table here and stop, leaving `src/` alone (the reproduction `tools/check.sh` compares with the committed table) |

## Adding a shader

1. Drop the GLSL in `tools/shaders/<name>.<vert|frag>`.
   - Uniforms → `layout(push_constant) uniform { … };` (→ `sh0..`, DOUTD).
   - Varyings → `layout(location=N) in/out …` (→ `VARYING_SLOT_VAR0+N`).
   - Attributes → `layout(location=N) in …` (→ `vi` regs in order).
2. Add a `src` line for it to `tools/shaders/precompiled.list` (or pass it on the
   command line), and a `pair` line for each program an application links it into.
3. `tools/glsl2usc.sh` — regenerates `tests/reference/blobs.rs`.
4. The app must `glShaderSource()` the **exact same source bytes**; the offline
   backend matches by SHA-256 of that text. The table's copy of the text
   (`SHC_SOURCE_TABLE`) always ends in a newline, so a source must end in one too.
5. Uniform names for `glGetUniformLocation` are read from the GLSL text: the
   push-constant block's members at std430 offsets (float/vecN/ivecN/bvecN/matN,
   arrays) and the sampler declarations with their `binding`. The block must be
   anonymous (`layout(push_constant) uniform U { ... };`); at most 255 uniforms.

### GLSL ES 1.00 / 3.00 sources

Applications written for OpenGL ES 2.0 pass GLSL ES 1.00 (`attribute`,
`varying`, plain `uniform`s, `gl_FragColor`, `texture2D`) to `glShaderSource`,
ES 3.0 applications GLSL ES 3.00.
`glsl2usc` accepts such files unchanged: its front end rewrites them into the
GLSL 450 the PCO pipeline compiles (attributes and varyings get locations in order
of appearance, uniforms — also struct-typed ones, flattened — become the
push-constant block, samplers get bindings, `gl_FragColor` becomes the location-0
output, cube maps and 2D arrays are sampled through stacked 2D textures) and the
table entry is keyed by the original text, so the application's source stays
byte-identical. Fragment varyings are linked to the vertex shader's outputs by
name at `glLinkProgram` (`shc_varying.name`), so a pair may declare them in any
order. Code outside comments must be ASCII (GLSL's character set).

### Blend and render-target variants

Blending and per-channel colour masks are lowered into the fragment shader on
this GPU, so `glsl2usc.sh` also compiles every fragment shader once per
`blend SPEC[:MASK][@RT]` line of `tools/shaders/precompiled.list` (or per spec of
the first `for GLOB ...` line whose glob matches its path). `SPEC` is
`srgb,drgb[,sa,da[,eqrgb[,eqa]]]` (names as `pco_shc`'s `PCO_SHC_BLEND`) or
`replace` (no blending); `MASK` is the channels `glColorMask` leaves on, a
subset of `rgba` (default all), e.g. `src_alpha,inv_src_alpha:rgb` or
`replace:ga`; `RT` packs the colour output for a float render target (`f16x4`,
`f16`, `f32`; `PCO_SHC_OUT`). A variant with a CONSTANT_* factor reads
`glBlendColor` from 4 dwords appended to the uniform block (`blend_consts_off`),
which the GL core fills at draw time. Add a spec there when an app uses another
combination; the runtime logs "no fragment shader variant for blend key 0x..." and
draws with the plain shader otherwise.

### Samplers

`layout(set = 0, binding = N) uniform sampler2D` compiles natively: the shader
reads a 20-dword combined image+sampler descriptor at `sh[20N..]`, which the
backend DMAs (see `SHADERC_NOTES.md`). The push-constant block follows the
descriptors.

### Internal shaders

`shaders/internal/` holds shaders the PowerVR backend binds itself (not looked up
by apps): `fbload.vert/.frag`, the framebuffer-load quad. `glsl2usc.sh` compiles
them with `yakogl-tools usc-blob` into `src/pvr/blobs.rs` (the `PVR_USC_FBLOAD*`
blocks, including the float render-target variants): the tool replaces a symbol's
block in place and leaves the rest of the file alone; with
`--after OTHER --title TEXT` a missing block is added after OTHER's.

### Shaders pco_shc can't compile (prebuilt sidecar)

For a shader the compiler cannot build, supply a `<name>.<frag|vert>.prebuilt.json`
sidecar next to the GLSL with known-good USC dwords + metadata; the tool hashes
the source and emits the entry verbatim (with the sidecar `note` as a comment).
No starter shader needs one any more (`texture.frag` used to). Sidecar schema:

```json
{
  "note": "why this is prebuilt",
  "stage": "fs",
  "temps": 32, "uses_w": 0, "uses_discard": 0, "num_outputs": 1,
  "uniform_dwords": 0, "vs_output_size": 0,
  "varyings": [ { "location": 0, "components": 2, "interp": 0, "vtxout_off": 4 } ],
  "attribs":  [],
  "usc": [ "0x....", "..." ]
}
```

A `<name>.<frag|vert>.override` next to a source is compiled in its place (still
keyed by the original bytes). See `src/shaderc/SHADERC_NOTES.md` for the metadata →
`ShcShader` mapping.

## femtovg-shaders — the FemtoVG sources

`yakogl-tools femtovg-shaders <femtovg-0.26.0/src/renderer/opengl> [outdir]`
writes the texts Slint's FemtoVG renderer assembles at run time (the vertex
shader and every `SELECT_SHADER` / glyph / `NO_DISCARD` permutation) byte for byte
into `tools/shaders/femtovg/`. The committed files come from the no_std fork's
patched `main-fs.glsl`.

## gl-consts — GL / EGL tokens for the Rust core

`yakogl-tools gl-consts` (no arguments) regenerates `src/gl/consts.rs` (GL tokens,
`u32`) and `src/gl/egl_consts.rs` (EGL tokens, `i32`) from the Khronos headers under
`include/`. Rerun it when those headers change.

## abi-layout — layout pins for the shared structs

`yakogl-tools abi-layout <powervr-bxs-am62p/src>` measures every `#[repr(C)]`
struct the C headers share with the crate (26 structs, 365 fields) with the host C
compiler (`CC`, default `cc`) and writes the numbers twice: `src/abi_layout_check.c`
(`_Static_assert`s) and `src/abi_layout.rs` (const assertions). A change to a header
or a Rust struct that is not mirrored on the other side fails the build; re-run the
tool when a shared struct changes. LP64 targets only.

## hostharness — host differential harness

Runs GL/EGL call streams through a library build on the host with a recording
platform layer (`rec.c`: all 17 OSAL operations, GPU memory hashes at every submit,
free and exit, decoded kicks) and an interposer on all 222 exported entry points
(return values, output parameters, read-back data): the driver repository's
`tools/pvrlab/gl_test.c` (`gltest.c`, 47 cases), API sweeps (`scen.c`) and hostile
sequences (`hostile.c`), each in twelve platform modes (lagging fences, no CPU view,
map_phys, uninitialised pools, a failing wait, the linux OSAL), under AddressSanitizer —
which covers the library itself, not only the harness.
`compare.sh <out> <libA> <libB> <driver checkout>` compares two libraries (dev and
release) and fails on any difference `allow.txt` does not explain; `run.sh` runs one
library; `report.py` evaluates runs. `hostharness/README.md` has the details and the
list of what is not compared. Note that a comparison against a C tree is no longer
expected to be quiet: the port deliberately diverges from upstream C where the C is
wrong (README.md, "Status"). The regression gate is the comparison against the previous
Rust build.

## vivante — the GC7000 shader tool and the command-stream differ

`tools/vivante/` holds two things for the i.MX8MP's Vivante GC7000, and both are
built inside a Mesa 26.2.1 tree with the etnaviv driver, which is not part of this
repository. `tools/vivante/README.md` has the setup and the measured status.

**The offline shaders.** `etna_shc.c` (a host tool in the Mesa tree, added by
`mesa-etna_shc.patch`) turns OpenGL-flavoured SPIR-V into GC7000 instruction words
plus the reflection the shader table needs; `glsl2vivante.py` rewrites GLSL ES 1.00
and 3.00 into the GLSL 4.60 it expects and reports what compiled. Its ES 1.00 parser
pieces live in `tools/vivante/es100.py`, a verbatim copy of the former Python
glsl2usc's. `gen_shader_table.py` turns that output into the generated table
(`tests/reference/vivante_blobs.rs` and `src/vivante/clear_shader.rs`).

**The command-stream differ**: what does a working driver actually tell this part?
Mesa's etnaviv runs on the host against its own drm-shim, patched by
`mesa-cs-golden.patch` to answer as the board's GC7000UL and to write out every
submit and the memory it points at instead of executing it. `cs_golden.c` draws a
shape through that stack and `cs_ours.c` draws the same shape, with the same GL
calls and the same shader sources, through this library; `cs_decode.py` decodes
either dump, naming registers out of Mesa's own `state_3d.xml.h` and following a
texture descriptor into the memory dump. The two decodes are then diffed. No GPU is
involved on either side, so the answer costs a second rather than a board run, and
it is a measurement rather than a fourth guess at a register — it is how the
fragment-uniform bug was found after three guesses had been spent on it.

Because the answer is only worth what its fidelity is worth, nothing here is
restyled: a changed constant or a changed dump format would destroy the diff.

`cs_ours.c` is the one file the port had to adapt, and not its source: in the C tree
the runner compiles the library's `.c` files, and here the library is Rust, so
`build_cs_ours.sh` builds the archive crate with cargo, appends the parts that are
still C with `ar` and links `cs_ours.c` against that — the same way
`tools/hostharness/run.sh` builds its C harnesses. It needs a driver-repository
checkout for `gl_test_shaders.h` and `pvr_render.h`.

# yakogl.ver — the shared libraries' exported ABI

`tools/yakogl.ver` is the linker version script of `libGLESv2.so.2` and
`libEGL.so.1`, the pair a Linux application (and the Khronos conformance suite)
links as `-lEGL -lGLESv2`. Only the Khronos entry points and
`gles_offline_shader_source` leave the library; everything else — the GL core,
the GPU backend, the OSAL, the shader table and the driver's command
generators — is `local`, so an application cannot bind to a name that is not
part of the API.

There is one copy of the code, not two. The GL core, EGL, the backend and the
OSAL share one process-wide state, so they all live in `libGLESv2.so.2`, which
exports the `egl*` entry points as well; `libEGL.so.1` is a forwarder that
carries the soname applications look for and holds one relocation into
`libGLESv2.so.2` so `--as-needed` keeps the `DT_NEEDED` entry and
`dlopen("libEGL.so.1")` still resolves `egl*`. Both are linked from the
assembled `libyakogl.a` with `--whole-archive`, so the archive and the shared
objects cannot drift apart.

Build them with `-DYAKOGL_SHARED=ON` for any OSAL (it needs
`YAKOGL_RUST_LINKAGE=staticlib`, the default; it is on by default for `gcgpu`).
The host `linux` OSAL builds the pair as a `libEGL` that stands in for an
application under ASan:

```sh
cmake -S yakogl -B build/so -DYAKOGL_OSAL=linux -DYAKOGL_SHARED=ON
cmake --build build/so          # → libGLESv2.so.2.0.0, libEGL.so.1.0.0, libyakogl.a
```

A board pair over `/dev/pvrgpu` (`YAKOGL_OSAL=pvrgpu`) is retired with its
kernel module (`driver/tools/pvrlab/README.md`). Conformance on the boards
runs in Zephyr dEQP images (`tools/cts/README.md`). A library built for a
conformance run still wants `-DYAKOGL_REPORT_ES2=ON`: without it dEQP reads
`GL_MAJOR_VERSION=3` and about seventy negative_api cases cannot pass, because
they require an error that an ES 3.0 context does not raise. Check which one a
built library is, rather than trusting the command line:

```sh
strings libGLESv2.so.2.0.0 | grep -E 'OpenGL ES [23]\.0 YakoGL'
```
# shader-check — the front end as a batch service

`yakogl-tools shader-check` answers one JSON request per line on stdin with one
reply per line on stdout:

```json
{"op": "compile", "stage": "vert", "source": "<GLSL ES text>"}
{"op": "link", "vert": "<GLSL ES text>", "frag": "<GLSL ES text>"}
```
```json
{"ok": true}
{"ok": false, "phase": "validate", "error": "ERROR: 0:5: '=' : cannot convert ..."}
{"error": "<the service is broken, not the shader>"}
```

`ok: false` means the application's shader is wrong; a bare `error` means this
tool is. Nothing merges the two: a caller that cannot tell them apart reports a
missing compiler as a conformance failure. `--codegen` also runs `pco_shc` (the
whole offline pipeline); `--legacy` reproduces the pipeline as it was before the
ES front end existed, for measurement only.

# es100_oracle.py — what dEQP-GLES2 expects of the front end

`tools/es100_oracle.py` parses dEQP's own shader cases
(`<VK-GL-CTS>/data/gles2/shaders/*.test`, ~2,800 sub-cases, each stating
`expect compile_fail | link_fail | pass`), asks `shader-check` for a verdict on
each, and reports how many expectations were met, per file, with the mismatches.
No GPU, no `pco_shc` and no dEQP build are involved.

```sh
tools/es100_oracle.py                        # everything, about three minutes
tools/es100_oracle.py --legacy               # the pipeline before the front end
tools/es100_oracle.py linkage --show-mismatches 20
tools/es100_oracle.py --cts ~/VK-GL-CTS --json out.json
```

The numbers, and the six cases that still do not match, are in
`src/shaderc/SHADERC_NOTES.md`.

### The two failure kinds, ruled

A shader the compiler rejected and a host that could not answer are different
replies and must never be confused. A name that does not fit the pool is a
deterministic property of the shader, so it comes back as
`{"ok": false, "error": ...}`, `GL_COMPILE_STATUS` false with the message in the
info log, and it is cached as a refusal; a service error (`{"error": ...}`, no
`"ok"`) is reserved for the host being unable to answer at all (a missing
compiler, an unreachable server) and is never cached.
