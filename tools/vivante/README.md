<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# YakoGL offline shaders for the Vivante GC7000 (i.MX8MP)

`etna_shc.c` is a host tool built inside a Mesa tree (26.2.1): it turns an
OpenGL-flavoured SPIR-V module into GC7000 instruction words plus the reflection
YakoGL's shader table needs, using Mesa's own etnaviv compiler and hardware
database (the GPU is described by model/revision/product id; no device is needed).
`glsl2vivante.py` rewrites GLSL ES 1.00 into the GLSL 4.60 the tool expects,
runs `glslangValidator -G` and `etna_shc`, and reports what compiled.

Set up once (the tree is not part of this repository):

```sh
cd <mesa-26.2.1>
cp <this dir>/etna_shc.c src/gallium/drivers/etnaviv/
patch -p1 < <this dir>/mesa-etna_shc.patch   # exports etna_get_specs, adds the meson target
meson setup build-etnaviv -Dgallium-drivers=etnaviv -Dvulkan-drivers= -Dplatforms= \
    -Dglx=disabled -Degl=disabled -Dgbm=disabled -Dllvm=disabled -Dtools=
ninja -C build-etnaviv src/gallium/drivers/etnaviv/etna_shc
```

Then `ETNA_SHC=<mesa>/build-etnaviv/src/gallium/drivers/etnaviv/etna_shc python3 glsl2vivante.py`.
Measured 2026-09-16, when the directory held 40 shaders (not re-measured on
today's larger set): all 40 shaders under `yakogl/tools/shaders` compile
(`40/40 compiled`); the matrix vertex shader is 4 instructions, FemtoVG's gradient
fragment shader 59, its image filter 369. The tool runs the GL state tracker's
preparation that the driver compiler expects but does not do itself: system
values to intrinsics, rectangle textures, `flrp` lowering. Missing any of those
shows up as an assertion in the driver's liveness pass or an unhandled ALU op.

## The shader table

`gen_shader_table.py` turns that JSON into the GC7000's Mesa reference table,
`tests/reference/vivante_blobs.rs` (kept for measurement; the product compiles
with our own compiler): one entry per shader under `tools/shaders`, found by the SHA-256 of the GLSL text the application
passes to `glShaderSource`, exactly as the PowerVR table works. It also writes
`tests/reference/gc_clear_shader.rs`, etna's two shaders for the backend's
clear quad (`tools/shaders/internal/clear.vert` and `clear.frag`): reference
material. The backend draws with our compiler's pair, `src/vivante/internal.rs`
(`yakogl-precompile --internal`, the list's `internal gc7000` lines), which
`tests/internal_programs.rs` holds to etna's word for word.

```sh
ETNA_SHC=<mesa>/build-etnaviv/src/gallium/drivers/etnaviv/etna_shc \
    python3 tools/vivante/gen_shader_table.py
```

`--append <path under tools/shaders>...` compiles only the named shaders and
adds them after the existing rows, leaving every committed row, symbol and
source entry byte for byte (the header names the toolchain that compiled the
appended rows). That is how a new shader gets a row on a host whose etna_shc
does not reproduce the committed table: this laptop's (e224bebc...) reproduces
none of the 324 rows, so a regeneration there would be a new reference. The
FemtoVG NO_SCISSOR programs were added this way on 2026-10-02, and on
2026-10-12 every table-directory source that compiles (219, issue #22; see
"The table against its sources" below). A fragment shader that reads
`gl_FrontFacing` is appended with its second row, as a full run places it.

`--replace <path under tools/shaders>...` recompiles named rows that are
already in the table, in place: each keeps its position and symbol number, and
its key (SHA-256) and stored source follow the file as it is now, so a row
whose source was edited after it was made gets the key the runtime computes.
Every other row stays byte for byte, and the header names the
toolchain that compiled the replaced rows. It refuses the result if any other
row changed. If no row changes at all, it writes nothing and says so. That is how a toolchain fix reaches only the rows it changes:
on 2026-10-04 etna_shc started lowering uniforms to UBO 0 as Mesa's GL state
tracker does (st_glsl_to_nir.cpp st_nir_lower_uniforms), so a run-time
index into a uniform array is a `load` from the stage's uniform block
(constant contents UBO_ADDR) instead of `movar` + a relative constant-file
read, which reads element 0 on the i.MX8MP. Compared on all 327 table
sources, the change moves exactly the four rows that held `movar`; two of
them (`es300/dynidx.frag`, `qt3d/e0880dcd5582d4de.frag`) were replaced, and
the two Qt particle vertex shaders are refused by the generator's layout
check (their `ubuf.sizetable` is packed in the GL core's block where Mesa's
offsets assume a vec4 per element) and kept `movar`. Those two rows left
with the Qt shaders (#1509), so no row reads a uniform array through `movar`
now (`no_table_row_indexes_a_uniform_array_through_the_address_register`).

`--remove <directory under tools/shaders>...` drops every row of those
directories: the row, its numbered arrays, its source entry and its header
listing lines. It needs no compiler. The rows that stay keep their bytes and
their symbol numbers, and it refuses the result if any other row changed.
That is how a directory leaves the tree: `qt/` and `qt3d/` left this way
(#1509).

`--st` compiles the committed table's rows (the stored sources, keys, blend
keys and order) for the STM32MP257F's GPU and writes
`tests/reference/vivante_blobs_st.rs`: the ids are the ones issue #1281 read
on the board (model `0x8000`, revision `0x6205`, product `0x80003`, eco `0`)
and customer `0x15`, the etnaviv hwdb row's (not read on the board; Mesa's
lookup matches all five, so `etna_shc` takes the eco and customer ids after
the three). The i.MX8MP table is never written by `--st`. `--st --check`
regenerates the ST table in memory and compares it byte for byte (exit 2
with a `gap:` line when there is no `etna_shc` at `ETNA_SHC`, under
`MESA_ETNA_TREE/build-etnaviv`, or at the default path). The ST table carries
no toolchain line, so the check is a byte comparison across hosts; the
generator prints the host's `etna_shc` digest and glslang version instead.
`tests/gc_st_blobs.rs` pins how many rows and instruction words the ids move
(91 rows, 5853 words of 234 rows: the hwdb row's `SH_NO_ONECONST_LIMIT`).

### The table against its sources (issue #22, 2026-10-12)

Issue #22 was "the GC7000 table lags the shader sources". Measured on the
main tip `ba25def7` (2026-10-13) before this change, by comparing each
source's SHA-256 with the row keys: the table had 330 rows (325 distinct
keys); of the 714 sources in the `gc7000` directories of
`tools/shaders/precompiled.list`, 326 had a row under their current key, 387
had no row, and one row (`es300/texsize.frag`) had a stale key, because a
precision line was added to the file after its row was made. The `matattr`
rows were current.

Generator fixes (all in this directory):

1. `glsl2vivante.py` `translate_es100_gl` translates three ES 1.00 forms that
   glslang refused before: `gl_FragData[0]` (now an `out vec4`), `textureCube`
   and `textureCubeLod`, and an array size written as a sum (`u_lights[0 +
   16]`, resolved in `es100.py` `_resolve_size`).
2. `etna_shc.c` makes `gl_PointCoord` a fragment input at `VARYING_SLOT_PNTC`,
   as the GL state tracker does, and splits matrix and array varyings per
   element. `gen_shader_table.py` names the PNTC row `gl_PointCoord`, the name
   the link looks for.
3. `--replace` rewrites the row's key and stored source (see above).
4. `--append` takes a fragment shader that reads `gl_FrontFacing` and appends
   both of its rows.
5. `check_gl_test_blobs.py` reports gl_test sources whose directory has no
   `gc7000` role (`gltest/`, `offarm/`) on a `LISTED OUT` line, not as
   `NO ROW`: leaving them out is the list's decision, not a lag.

The commands and what they printed (etna_shc built from this directory's
`etna_shc.c` in Mesa 26.2.1 as "Set up once" above says, binary sha256
`c9845f2b...`; glslang 16.2.0; the table's own header lines name both):

```sh
ETNA_SHC=<mesa>/build-etnaviv/src/gallium/drivers/etnaviv/etna_shc \
    python3 tools/vivante/gen_shader_table.py --append <the 219 sources>
  -> kept 330 committed rows byte for byte, appended 220
ETNA_SHC=... python3 tools/vivante/gen_shader_table.py --replace es300/texsize.frag
  -> kept 549 committed rows byte for byte, replaced 1: es300/texsize.frag
python3 tools/vivante/gen_shader_table.py --self-test
  -> allowlist self-test ok (11 directories accounted for)
     550 committed rows parse; row comparison ok
python3 -I tools/vivante/check_gl_test_blobs.py        (exit 0)
  -> table tests/reference/vivante_blobs.rs: 544 rows  (distinct keys)
     gl_test_shaders.h  192 shaders, 0 without a row, 12 with an empty
       program, 82 left out by the source list, 3 refused by the compiler
         LISTED OUT 69 from tools/shaders/gltest/, 13 from tools/shaders/offarm/
         REFUSED  gl_test_uniidx_a_fs_src, _b_, _c_  (layout check)
     gl_test_shaders_es100.h  the same
     glsl2usc.sh inputs 587 sources, 0 stale, 32 excused
ETNA_SHC=... python3 tools/vivante/gen_shader_table.py      (the full run, exit 1)
  -> FAIL <source> <message>, one line per source the toolchain refuses
     (176: the 168 below, the three `matattr` and two Qt particle vertex
     shaders whose rows an older etna_shc made, `gltest/isamp.frag`,
     `gltest/usamp.frag` and `internal/es/fbload_int.frag`);
     reproduced 56 of 550 committed rows byte for byte; REFUSING to overwrite
```

The full run is the enumeration of what the toolchain refuses, not a
regeneration: this host's etna_shc does not reproduce the rows an older build
made, so the generator refuses to overwrite the reference (that is its design,
see "THE COMMITTED TABLE IS THE REFERENCE" in `gen_shader_table.py`). "The 219
sources" are the table-directory sources without a row that the full run does
not FAIL; `es100/frontfacing.frag` brings its `@front_cw` row, so 220 rows were
appended. The same 220 rows were made twice, on two hosts with different builds
of etna_shc (`c9845f2b...` with glslang 16.2.0, which is the committed table,
and `1d52afef...` with glslang 16.4.0): the two tables differ only in the two
header lines that name the toolchain.

After it: 550 rows under 544 distinct keys (the check counts keys): the four
`@front_cw` rows share their shader's key, and two pairs of files have the
same text, so one row per file gives 546 sources but 544 keys
(`qt3d/dd68cd85f59b3ec6.vert` = `qt3d/offlineshaders_es3_qtappshaders_1.vert`,
`qt3d/helloqtquick3d_qtappshaders_0.frag` = `qt3d/qtappshaders_0.frag`). Every one of the 714 sources in a `gc7000` directory
either has a row under its current SHA-256 (546), or is refused (168); no row
has a stale key, and the only rows with no file of their own are the four
`@front_cw` rows. The 168 refused sources, each compiled again and its message
read:

| Class | Count | Sources | Message |
|---|---|---|---|
| layout check | 3 | `es100/uniidx_a/b/c.frag` | a run-time uniform index reads block dword 4 or 1 where Mesa's offsets point at 16 or 4 |
| constant file | 2 | `es100/light_amount_0_16.vert`, `light_amount_2_2.vert` | constant register 44 reads uniform index 0, which no declared uniform covers |
| Mesa assertion | 2 | `es100/vtex_struct_sampler(_array).vert` | `nir_lower_samplers.c:45` (a sampler in a struct) |
| etna_shc assertion | 14 | 13 `qt/*.vert`, `qt3d/27371573a9ac1abd.frag` | `etnaviv_compiler_nir_liveness.c:94` |
| reflection | 83 | 51 `qt3d/<hash>.frag`, 32 `qt3d/qtparticleshaders_*.frag` | nested structs are not reflected |
| glslang | 64 | `qt3d/qtparticleshaders_*.vert` | glslang: SPIR-V is not generated for failed compile or link (the earlier record read `_94.vert`'s cause: `num_views` needs an extension; the others were not read one by one) |

Only `uniidx_a/b/c` are gl_test sources. `check_gl_test_blobs.py` names them
in `GC7000_REFUSED` with this message and reports them as `REFUSED`, not as a
lagging table, so it exits 0; a listed file that is edited (its SHA-256 no
longer matches) or that gains a row fails the check again. A row for them
needs the GL core's uniform block to match Mesa's offsets, or a generator
that rewrites the offsets.

The table is the corpus of several host tests, so their population pins
moved with it (`alloc_clobber`, `backend_differential`,
`lane_move_equivalence`, `undefined_lanes`, `operand_banks`,
`online/vivante/roundtrip.rs`, `es100_twins`, `source_use_bits`). When the
table changes again, take the new numbers from the failing tests' messages.

Measured again on main `5466c8dc` (2026-10-13), the same SHA-256 comparison:
neither the table nor `tools/shaders/` changed since `01fbbf17`, and the
counts are the same: 550 rows, 544 keys; 714 sources, 546 with a current row,
0 with a stale key, 168 without a row, and those 168 are exactly the six
classes above (3 + 2 + 2 + 14 + 83 + 64).

### The Qt shaders leave (issue #1509, 2026-10-24)

`tools/shaders/qt/` and `qt3d/` left the tree (their text is Qt's). Their
342 rows left the table with `--remove qt qt3d`; the other 208 rows stayed
byte for byte. The test sources written for the tests that used the Qt
shaders (`es100/xlink`, `sharedint`, `texfill`, `fmods`, ...; `es300/mrt2`,
`gcops`, ...) were appended, 26 rows, compiled by the etna_shc the header
names: 234 rows. The six classes above were measured with the Qt sources
in the tree; their counts were not measured again (the three etna_shc,
reflection and glslang classes were Qt sources only). Every appended
source compiled.

### The `matattr` rows (issue #22, settled 2026-10-14)

Issue #22 first said "the GC7000 table lags on `matattr`". The keys of the
four rows (`es100/matattr.frag`, `matattr2.vert`, `matattr3.vert`,
`matattr4.vert`) match the files, and `check_gl_test_blobs.py` finds them, so
gl_test case 63 has no table miss. The other half of the question was
whether the code in the rows is what today's etna_shc makes. The first
answer was that etna_shc could not remake the three vertex rows, and the
cause was in this directory, not in the shaders. That cause is now fixed
(below):

- The committed rows describe a matrix attribute as one input: `a_xf` is
  one `ShcVivIo` at slot 16 (location 1), `vi_reg` 1, and the code reads
  the columns from registers 1, 2, ... That is the shape the runtime
  expects: `src/vivante/shader.rs` puts column k of a matrix attribute in
  register `vi_reg + k`, one row per attribute. [read]
- `etna_shc.c` called
  `nir_lower_io_array_vars_to_elements_no_indirects(nir, false)`, added to
  split matrix and array varyings (fix 2 above). With `outputs_only` false
  it also splits shader inputs, and in a vertex shader the inputs are the
  attributes. Mesa's pass splits matrices into columns and puts column k at
  `location + k` (`nir_lower_io_array_vars_to_elements.c`, `lower_array`
  and `nir_lower_io_array_vars_to_elements_no_indirects`, Mesa 26.2.1).
  [read]
- So `a_xf` comes back as slots 16, 17, ... Slot 17 is location 2, which
  no declaration names, so `reflect` falls back to the raw slot and stops
  with "vertex input slot 17 is not a generic attribute"
  (`gen_shader_table.py`, the attribute loop). [measured 2026-10-14,
  etna_shc `1d52afef...` built from this directory's `etna_shc.c`,
  glslang 16.4.0: `glsl2vivante.compile_one` compiles all four sources;
  `reflect` refuses `matattr2.vert`, `matattr3.vert` and `matattr4.vert`,
  each with "vertex input slot 17 is not a generic attribute";
  `matattr.frag` reflects and its row is the committed one byte for byte]
- `matattr.frag` reads one `vec4` varying and is a pass-through program
  (`check_gl_test_blobs.py` reports it as EMPTY); the split does not
  touch it.

The committed rows are kept: they are the shape the runtime uses, and
the dEQP images do not read this table. The fix is in the tool:
`etna_shc.c` now keeps vertex attributes whole. It passes `outputs_only`
true for the vertex stage, so only varyings are split. That is what Mesa's
PowerVR compiler does (`pco_nir.c`:
`nir_lower_io_array_vars_to_elements_no_indirects(nir, stage ==
MESA_SHADER_VERTEX)`). Fragment shaders are compiled as before.

[measured 2026-10-14, etna_shc `87ecbb38...` built from this directory's
`etna_shc.c` with that change, in a copy of the same Mesa 26.2.1 tree,
glslang 16.4.0]

- `reflect` accepts all four `matattr` sources, and their rows are the
  committed ones byte for byte: `replace_rows` with the four new rows
  changes no row. Only the header would change, by naming the toolchain.
  `--replace` used to stop here with "the splice changed other rows
  (changed [] ...)", although nothing had changed. It now prints "all N
  asked rows already match the committed table; not written" and exits
  0, leaving the table as it is.
- Every `.vert` under `tools/shaders` except `internal/` (324 sources)
  was compiled with the old binary (`1d52afef...`) and the new one. 306
  results are the same. 3 differ: `matattr2/3/4.vert`, refused before and
  accepted now. In the other 15, the same source fails with the same Mesa
  assertion in both, and the message differs only in the Mesa tree's path
  (`vtex_struct_sampler(_array).vert`, 13 `qt/*.vert`).

Still open after this change:

- `check_table_absent` under LTO is still open (later work).
- `gltest/` (69 gl_test sources) and `offarm/` (13, PowerVR only) have role
  `-` in `precompiled.list`, so they get no GC7000 row. Giving `gltest` the
  `gc7000` role needs an i.MX8MP gl_test run with no compiler afterwards.
- The 168 refused sources above.
- No new row has run on a board. The table is the Mesa reference
  (`shaderc::offline`): host tests and the `mesa-reference-table`
  measurement images use it. The dEQP images use the compiler, so no
  dEQP-GLES2 result can move because of it. `src/shaderc/precompiled.rs` (our
  compiler's table, made per build) cannot lag the sources and needed no
  change.

Two differences from `tools/glsl2usc.sh`, which builds the PowerVR table:
blending and colour masks are pixel-engine state on this GPU, so there are no
blend variants and every fragment shader appears once; and the input list is
the whole `tools/shaders` tree rather than a hand-kept order, because there is
no variant matrix whose order has to stay stable. A shader the toolchain
cannot build is reported and left out rather than failing the table.

`glsl2vivante.py` translates GLSL ES 1.00 and GLSL ES 3.00; anything else is
taken to be the GLSL 450 the PowerVR tool is fed. The ES 3.00 rewrite is the
small one: those sources are already written the modern way, so it raises the
version to 4.60 and gives a location or a binding to each interface
declaration that has not got one, which is what OpenGL SPIR-V insists on.
Locations need not agree between the stages -- the link matches varyings by
name, which is what GLSL ES means by a varying -- but within a stage they must
be unique, so the ones written by hand are read first and stepped over.

Measured 2026-09-21: 326 of 556 sources compile, against 128 before the ES
3.00 path, and the table holds 304 entries against 138. All twelve of
`gl_test`'s ES 3.00 shaders are in it, matched by SHA-256 against the sources
the suite passes to `glShaderSource`. What is still out is the rest of the Qt
Quick and Qt Quick 3D sweeps. `src/vivante/VIVANTE_NOTES.md` documents the ABI
a table entry holds.

Output per shader (JSON, then the disassembly): `code` (instruction words),
`num_temps`, `inputs`/`outputs` (register, gl slot, components), `uniforms` (the
constant register file: contents 2 = a GL uniform component, `data` = its index),
`uniform_names` (name, GL location, vec4 slot / texture unit), `vs_pos_out_reg`,
`ps_color_out_reg`, `input_count_unk8`, `uses_discard`. What the driver-side
backend does with these is `src/vivante/VIVANTE_NOTES.md`, "The shader ABI".

## The golden command stream: what a working driver tells this part

`cs_golden.c` + `cs_decode.py` + `mesa-cs-golden.patch` answer the question
that reading somebody else's source cannot: **what does the hardware actually
get told?** Mesa's etnaviv driver runs on the host against its own drm-shim,
which answers as the board's GC7000UL and keeps every submit instead of
executing it. No GPU is involved, so the answer costs a second rather than a
board run, and it is not an opinion.

This is the tool to reach for before guessing at a register. It is how the
fragment-uniform bug was found, after three guesses had been spent on it.

### Building it

```sh
cd <mesa-26.2.1>
patch -p1 < <this repository>/tools/vivante/mesa-cs-golden.patch     # see "what the patch does"
meson setup <build dir>/mesa-etnaviv-host \
    -Dgallium-drivers=etnaviv -Dvulkan-drivers= -Dtools=drm-shim \
    -Degl=enabled -Dgles1=disabled -Dgles2=enabled -Dglx=disabled \
    -Dplatforms= -Dgbm=disabled -Dllvm=disabled -Dvideo-codecs= \
    -Dshared-glapi=enabled -Dglvnd=disabled -Dbuildtype=debugoptimized
ninja -C <build dir>/mesa-etnaviv-host
```

`-Dglvnd=disabled` is what gives a standalone `libEGL.so`/`libGLESv2.so` to
link against instead of a GLVND vendor library.

Meson also wants the Python `pycparser` module (>= 2.20) for the etnaviv hwdb.
Built with the commands above, the patched 26.2.1 tree compiles cleanly (1022
of 1022 steps, 2026-11-06), with both the two-entry patch of `1673798c` and the
three-entry patch in this directory.

### Running it

The commands below call two runner scripts, `build/run_cs_golden.sh` and
`build/run_cs_ours.sh`, that lived in the gitignored `build/` of the
checkout they were written in; they are not in the repository. Our side
is built by `tools/vivante/build_cs_ours.sh`, and its header shows how to
run `cs_ours` and decode its dump with `cs_decode.py`. The golden side is
`cs_golden.c` built against the Mesa tree above. Both take the mode number
first (the list is in `cs_ours.c`, "the mode numbers, in one place").

```sh
build/run_cs_golden.sh 1                  # gl_test case 1's shape
build/run_cs_golden.sh 1 --uniforms-only  # only the constant-file writes
build/run_cs_golden.sh 2                  # a vertex uniform as well
build/run_cs_golden.sh 4                  # a textured quad (gl_test case 2)

build/run_cs_golden.sh 8                   # a depth-tested draw (gl_test case 3)
build/run_cs_golden.sh 12                  # the same, cleared to depth 0.5 (case 13)
build/run_cs_golden.sh 14                  # case 3's five frames, one submit each
build/run_cs_golden.sh 15                  # a depth clear with nothing drawn behind it

build/run_cs_ours.sh 4 --final-state      # the same draw, this backend
```

`run_cs_ours.sh` builds YakoGL for the Linux OSAL and runs `cs_ours.c`, which
makes the same GL calls with the same shader sources, so the offline table's
SHA-256 lookup finds the shaders the board runs. Neither script needs a GPU.

A good golden run prints `renderer: Vivante GC7000 rev 6204` and `drew; gl
error 0x0` before the decode. `cs_decode.py` names registers out of Mesa's own
`state_3d.xml.h`, so the names in a diff are the ones its source uses; point
`--regs` elsewhere if that tree moves. Every line carries the register's
address as well as its name, because banks overlap and a name alone has been
wrong before.

Names come from **two** of Mesa's headers. `state_3d.xml.h` does not carry the
GL block -- `GL_SEMAPHORE_TOKEN`, `GL_STALL_TOKEN` and `GL_FLUSH_CACHE` are in
`state.xml.h` -- and that is the bank a hang is read in, so it was the worst
one to be printing bare addresses for. Adding it (`--regs2`) named eighteen
more registers and, more to the point, corrected three that were being named
*wrongly* by a neighbouring bank's array:

| address | was printed as | actually |
| --- | --- | --- |
| 0x17800 | `NTE_DESCRIPTOR_SAMP_ANISOTROPY[256]` | `NFE_GENERIC_ATTRIB_CONFIG0[0]` |
| 0x17a00 | `NTE_DESCRIPTOR_SAMP_ANISOTROPY[384]` | `NFE_GENERIC_ATTRIB_SCALE[0]` |
| 0x17a80 | `NTE_DESCRIPTOR_SAMP_ANISOTROPY[416]` | `NFE_GENERIC_ATTRIB_CONFIG1[0]` |

So vertex-attribute state was being read as texture-sampler state. Any diff of
attribute registers taken before 2026-09-22 is worth re-reading. The values
were never wrong -- only the names -- which is exactly what makes it dangerous.

### How the two drivers clear depth, and why the pair 8 / 12 exists

Mode 8 is a depth-tested draw and mode 12 is the same draw with
`glClearDepthf(0.5)` in front of it. Running each against both drivers pins
down the one thing a single mode cannot: whether the clear *value* reaches the
depth buffer, as against the depth state merely looking right.

Mesa does not draw a quad for the depth clear. It clears through the resolve
engine, and the value is a literal in the stream, so diffing its two modes
shows it directly:

```
 mode 8    RS_CLEAR_CONTROL  count=5  0x0001eeee 0xffffff00 ...
 mode 12   RS_CLEAR_CONTROL  count=5  0x0001eeee 0x80000000 ...
```

That is the stored D24S8 word, depth in the top 24 bits and stencil in the
bottom 8: `0xffffff` for 1.0 and `0x800000` for 0.5. It is worth writing down
because it is the ground truth for what the buffer should hold, independent of
any theory about the z transform.

This backend clears with a full-viewport quad instead, at
`z = clear_depth * 2 - 1` in clip space with the depth func forced to ALWAYS.
So its two streams are identical -- the clear value is in a vertex buffer, not
in a register -- and the difference shows up only in the memory dump:

```
 mode 8    -1 -1  1  1   1 -1  1  1   -1  1  1  1   1  1  1  1
 mode 12   -1 -1  0  1   1 -1  0  1   -1  1  0  1   1  1  0  1
```

Measured 2026-09-21: both sides are as intended, and the viewport z state the
two drivers program is identical (`PA_VIEWPORT_SCALE_Z` 1.0,
`PA_VIEWPORT_OFFSET_Z` 0.0). Mesa's own comment in `etnaviv_state.c` explains
that pair -- on this part the hardware's viewport z transform is 0..1 to 0..1,
and Mesa folds the -1..1 conversion into the scale and offset it programs, so
1.0 / 0.0 is what the default GL depth range comes out as. Note also that
`nir_lower_clip_halfz` is *not* applied for this GPU: `vs_need_z_div` is
`model < 0x1000`, and the model here is 0x7000.

**Read that second fact carefully, because I did not.** It says the *shader*
does not do the halving. It does not say nothing does. **The hardware does**,
measured on the board 2026-09-22: a depth clear whose quad carried 0.25
directly stored 0.625, which is `(0.25 + 1) / 2` (gl_test case 53, hold code
39; identity would have given 15). So clip z arriving at this part is in
-1..1 and it converts, and anything drawn here -- including an internal clear
quad with a NOP vertex shader -- must arrive in clip space like geometry.

I built a root cause on the difference between "the shader does not" and
"nothing does", and no amount of reading Mesa settled it: both readings are
consistent with every line of its source. One board reading settled it in
four minutes.

`glDepthRangef` is honoured, and by the same algebra Mesa uses, which is worth
writing down because the two expressions look nothing alike. Mesa programs
`SCALE_Z = scale[2] * 2` and `OFFSET_Z = translate[2] - scale[2]` from a
gallium viewport, where a GL range [n, f] gives `scale[2] = (f - n) / 2` and
`translate[2] = (f + n) / 2`. Substitute and they are `f - n` and `n` -- which
is exactly what this backend writes. For the default [0, 1] both come out as
1.0 and 0.0, the pair in every diff above. Case 13's check 10 uses a
non-default range, so this is the part it exercises.

The upshot is that the host side does not explain gl_test case 13's first
check, which is exactly `glClearDepthf(0.5)` being honoured and which fails on
the board. Whatever is wrong is not in the stream, not in the clear quad, and
not in the viewport state.

### Several frames in one mode, and what that caught

Mode 14 replays gl_test case 3 in full: the depth-tested frame, then the
depth test turned off, then `GL_GREATER` with `glClearDepthf(0)`, then the two
polygon-offset frames. Five frames, five submits, so the decode shows not only
each frame's state but what changes between them -- which is the half a
single-frame mode cannot reach.

It earned its keep immediately. Frame 2 turns the depth test off and leaves
`glDepthMask` alone, and the two drivers disagreed:

```
 Mesa   PE_DEPTH_CONFIG  0x05040711     depth func ALWAYS, write bit CLEAR, DISABLE_ZS
 ours   PE_DEPTH_CONFIG  0x00041711     depth func ALWAYS, write bit SET
```

With `GL_DEPTH_TEST` disabled the depth buffer is not written at all, whatever
the mask says; this backend was writing it. Fixed in `emit_pe_state`, after
which frame 2 reads `0x01040711` and the only bit still differing from Mesa is
`SUPER_TILED`, which describes Mesa's depth surface and not ours.

Two things the mode confirms rather than contradicts, both worth having
pinned: the polygon-offset units come out bit-identical to Mesa's
(`SE_DEPTH_BIAS` 0xb5000001 for -4 and 0x35000001 for +4, i.e. `units * 2 /
(2^24 - 1)`), and Mesa's `glClearDepthf(0)` is an RS clear of a literal
`0x00000000`, the same mechanism as the 0.5 clear in mode 12.

### Adding a mode

Not every mode exists on both sides, and that used to be dangerous rather than
merely inconvenient. `cs_ours` had no modes 2 or 3, and `cs_golden` has no 6,
9, 13 or 16 -- and each harness quietly drew **mode 1's shape** instead. So
`run_cs_ours.sh 2` against `run_cs_golden.sh 2` compared our mode-1 stream
with Mesa's real mode 2, and the diff showed a vertex uniform we were
"missing": a driver bug that does not exist, manufactured entirely by the
tool. Both harnesses now refuse a mode they do not implement and print the
list of the ones they have.

A refusal is not always the right guard, though, and a central list cannot
know that. `cs_golden` *does* have a mode 16 branch -- what it does not have is
a context that will accept ETC2, so ES 2.0 rejects the upload with
`GL_INVALID_ENUM`, the binding keeps its default texture and the harness would
happily describe that instead (`SIZE 0x00010001`, `FORMAT 7`, no compression).
A plausible golden that describes nothing is worse than a refusal, because
nothing stops it being diffed against. So that branch checks `glGetError`
after the upload and stops there, naming the reason. The rule this points at:
**a central list can only say whether a branch exists; only the branch knows
whether it can produce a valid answer**, so the guard belongs where the
knowledge is.

Both harnesses therefore answer in their exit code, which is what a script
should test -- the wording is not a contract:

| code | meaning |
| --- | --- |
| 0 | the mode ran; the stream is worth diffing |
| 2 | no such mode in this harness (the other one may have it) |
| 3 | the branch exists but could not produce a valid stream; the reason is printed |

Grepping stderr for a phrase instead is how a check comes to report success for
every mode including the ones that refuse. That happened here, once.


The mode numbers are defined once, in `cs_ours.c`'s "the mode numbers, in one
place" list. Add the row there before writing the code, and take the lowest
free number: the list is kept in order so that a gap is visible.

This matters more than it sounds. Mode 15 was claimed twice in one evening --
a depth clear and an ETC2 texture -- by two people whose commits touched
different lines of the same two files, so git merged them with no conflict and
mode 15 quietly did both things. Neither reading would have meant anything and
nothing would have said why. A textual conflict would have been the kind
outcome.

### A mode whose answer was that there was no stream

Mode 15 is a depth clear with no draw behind it: `glClearDepthf(0.5)`,
`glClear(GL_DEPTH_BUFFER_BIT)`, `glFinish()`. GL has to carry that out at the
finish. Mesa emits a 44-word submit clearing depth to `0x80000000`. This
backend emitted **no command stream at all** -- the dump file was not even
created, which is how the bug announced itself.

`gl_flush_pending_clear` was gated on the colour clear alone, so a depth- or
stencil-only clear stayed pending and was applied to whatever draw came next,
however much later. Fixed in the C-era GL core (`src/gl_draw.c` then; the
pending clear is in `src/gl/draw.rs` today), with the colour mask following
the colour clear so that a depth-only clear leaves the colour buffer alone
(`PE_COLOR_FORMAT` 0x00000006 rather than 0x00010f06: the write-mask field is
zero).

Every other mode's stream is byte-identical across that fix, which is the
check worth running whenever the clear path is touched:

```sh
for m in 1 4 8 12 14; do build/run_cs_ours.sh $m | sed 1,2d > before/o$m.txt; done
# ... change something ...
for m in 1 4 8 12 14; do build/run_cs_ours.sh $m | sed 1,2d | diff before/o$m.txt -; done
```

A missing dump now prints `no command stream was submitted` rather than a
traceback, because it is a result and not a broken tool.

### Which differences in a depth-mode diff are expected

A diff of modes 8 / 12 / 14 still shows two dozen registers apart. Most of
them are not bugs, and knowing which is which is the difference between a
diff that is useful and one nobody trusts. Per submit, the expected ones:

| register | why it differs |
| --- | --- |
| `PE_PIPE_COLOR_ADDR`, `PE_PIPE_DEPTH_ADDR` | the two drivers allocate in different heaps; only the slot and the contents compare |
| `PE_PIPE_*_ADDR[1]` | Mesa writes pipe 0 only; we write both, which is inert on a one-pipe part |
| `PA_VIEWPORT_SCALE_Y` | ours is negative: the window surface is y-flipped, Mesa's FBO is not |
| `PE_COLOR_FORMAT` tiling bits | Mesa's colour surface is tiled, ours is linear |
| `PE_LOGIC_OP` dither field, `PE_STENCIL_CONFIG_EXT+2/+3` | Mesa dithers and carries a dither table; we do not dither |
| `PE_MEM_CONFIG`, `TS_*`, `RS_*` | Mesa's tile status and resolve-engine clears, which this backend does not use |
| `PE_STENCIL_*` | we program stencil on every draw; Mesa only when it is enabled |
| `PE_DEPTH_CONFIG` `SUPER_TILED` | describes Mesa's depth surface, not ours; setting it here was measured inert on the board |
| `NFE_VERTEX_STREAMS_CONTROL` stride | we repack, Mesa does not. `cs_ours`'s vertex struct is 32 bytes but a mode using only position and uv fills 24, and the GL core converts such an array into a tightly packed scratch buffer -- so our 24 describes our buffer and Mesa's 32 describes the application's. What has to agree is the *offset* between streams, and `BASE_ADDR[1] = BASE_ADDR[0] + 16` on both. (vivante-vertex2, mode 10) |
| `FE_INDEX_STREAM_CONTROL`, `FE_PRIMITIVE_RESTART_INDEX`, `FE_HALTI5_ID_CONFIG` | Mesa writes all three as zero, we do not write them at all; zero is the reset value. Newly visible only because the GL block gained names -- worth revisiting if anything ever uses primitive restart |
| the clear quad's z | ours carries the clear depth itself; Mesa has no clear quad at all, clearing through the resolve engine instead. The two are not comparable, and the value is in a buffer rather than the stream, so no diff reaches it -- see the intermediate-clear note above |
| `GL_FLUSH_CACHE` 0x03 (DEPTH \| COLOR) | Mesa emits it only ahead of an `RS_KICKER`, to make the resolve engine see what the pixel engine wrote. We clear with a quad through the same pixel engine, so there is no gap to close and nothing missing |

The colour format of the golden's render target used to be on that list for
the wrong reason. It was `GL_RGBA4`, and Mesa derives both `PE_COLOR_FORMAT`
and `PE_LOGIC_OP`'s `SINGLE_BUFFER` from the colour format, so a 16-bit target
put a difference into those two registers that had nothing to do with depth.
The depth modes now use `GL_RGBA8` so that the comparison is like for like;
every other mode keeps `GL_RGBA4`, so readings already taken against it stand.

What is left after all that, and still open:

- **early Z.** Mesa runs these draws early (`RA_EARLY_DEPTH` 0x05000031,
  `PE_DEPTH_CONFIG` with `EARLY_Z` and `DISABLE_ZS`); we run them late
  (0x15000030, `WRITE_DISABLE` set). Late is internally consistent and Mesa
  emits that same combination itself when early is not allowed, so it is a
  difference rather than a known fault.
- **`PE_LOGIC_OP`'s `SINGLE_BUFFER`**: 2 from Mesa, 1 from us. Mesa picks 1
  only for a linear target and 2 otherwise, and ours *is* linear, so 1 may
  well be right -- but it has not been tested on the board either way.

### The blocks the stream only points at

A command stream is half of what the GPU reads. A texture descriptor, a shader
binary, a vertex buffer: the stream carries an address and the hardware
fetches the block from memory. A stream diff is blind to all of it, which is
how a wrong 256-byte texture descriptor hides behind a set of registers that
match perfectly.

**And a dump you read against your own code is not evidence either.** This is
the sharper half of the lesson, learned the hard way on 2026-09-22. A
`glClearDepthf(0.5)` was landing in the depth buffer as 0.0. The command
stream was *correct* -- every depth register matched Mesa -- because the clear
value never appears in a register at all: this backend clears with a quad, so
the value is four floats in a vertex buffer. The stream diff was structurally
incapable of showing it.

The memory dump did show it, twice, and it was read as correct both times. The
quad carried clip-space `0.0` for a 0.5 clear, which is exactly what
`clear_depth * 2 - 1` computes -- so it matched the code, and the code was
what was wrong. Checking a dump against the source that produced it only
confirms the source ran. What settled it was a board reading of the value the
hardware actually stored (`gl_test` case 53, which clears depth and does
nothing else), and that took one run where four hours of diffing had taken
none.

So: **the stream says what the hardware is told, the dump says what the driver
wrote, and neither says what the hardware did.** When those two agree and the
board still disagrees, stop diffing and go and measure.

So both sides dump their memory beside the stream, in one format: per
allocation a device address, a size, a zero, the bytes, padded to 8. Mesa's
shim writes it to `ETNA_SHIM_MEM_DUMP`, this backend's Linux OSAL to
`YAKOGL_MEM_DUMP`, and both runner scripts set them.

```sh
build/run_cs_golden.sh 4 --tex-desc       # Mesa's texture descriptor
build/run_cs_ours.sh   4 --tex-desc       # ours, field by field
```

`--tex-desc` follows every `NTE_DESCRIPTOR_ADDR` into the dump and prints the
descriptor named from Mesa's `texdesc_3d.xml.h` -- the same header its
`etnaviv_texture_desc.c` fills the block through, so the two sides are named
identically. The addresses themselves differ (the two drivers allocate in
different heaps); the slot number and the contents are what compare.

### Another part: the ST row

The shim has a third GPU, `ETNA_SHIM_GPU=GCNANOULTRA31`: the STM32MP257F's
(model `0x8000`, revision `0x6205`, product `0x80003`, eco `0`; issue #1281,
Q1). The customer id was **not read** from the board; the entry uses `0x15`,
the value of the etnaviv hwdb row for these ids (the hwdb has two such rows,
eco 0 and eco 1; eco 0 is the one matched). It sits after `GC7000UL`, so the
default and every existing golden are unchanged. The name must not be a prefix
of another entry's (names are matched by prefix).

Measured on 2026-11-06 with Mesa 26.2.1, the default build recipe above, and
`cs_golden` (modes 1-5, 7, 8, 10-12, 14-27, plus the new mode 28; mode 16 exits
3 by design on every row).

**The old rows are unchanged.** For every mode, the `ETNA_SHIM_CS_DUMP` and
`ETNA_SHIM_MEM_DUMP` files of the tree patched with `1673798c`'s patch and of
the tree patched with this one are byte-identical, both with no
`ETNA_SHIM_GPU` and with `ETNA_SHIM_GPU=GC7000UL` (24 modes, plus mode 16 whose
dumps are both empty).

**The driver runs on the new row.** Every mode exits 0 (16: 3, as on the other
rows), prints `renderer: Vivante GC8000 rev 6205` and `gl error 0x0`. The row is
also `CORE_NPU`; the `get-param (1) failed` line that the GC7000UL row prints
while probing core 1 does not appear on it (why was not traced).

The streams have the same shape on both rows, same submits, same command and
`LOAD_STATE` counts in every mode (`cs_decode.py`, GC7000UL against
GCNANOULTRA31):

| mode | submits | commands | differing state writes | memory dump |
| --- | --- | --- | --- | --- |
| 1, 2, 4, 7, 10, 11, 17, 18, 19, 20, 22, 23, 24 | 1 (11: 7, 17: 8) | 179-406 | 2 | identical |
| 3 | 2 | 358 | 4 | identical |
| 5 | 1 | 381 | 4 | identical |
| 8, 12 | 1 | 192 | 2 | identical |
| 14 | 5 | 948 | 10 | identical |
| 15 | 1 | 18 | 0 | identical |
| 21 | 2 | 424 | 4 | identical |
| 25 | 1 | 179 | 3 | **differs** |
| 26, 27 | 1 | 900, 904 | 8 | identical |
| 28 | 1 | 179 | 4 | identical |

Every difference but two is the same pair of registers:

- `PE_MEM_CONFIG` (0x014bc): the GC7000UL writes `COLOR_TS_MODE` (bit 24, and
  `DEPTH_TS_MODE` bit 26 where there is a depth buffer: 0x01000000 and
  0x05000000); the ST row writes 0 -- the same bits, off.
- `TS_MEM_CONFIG` (0x01654): the GC7000UL writes colour compression
  (0x80, plus 0x40, 0x4000 and a format in 0x300 with depth and stencil,
  e.g. 0x43c2 in mode 8, 0x40c2 in mode 22), the ST row writes only the colour fast clear (0x02,
  or 0x00 for the mipmapped textures of modes 5, 26 and 27, which the
  GC7000UL gives 0x380/0x480).

So Mesa does not use tile-status compression on the ST row; nothing else in
the common state moves.

**Modes 2 and 23 (`SH_NO_ONECONST_LIMIT`) show no difference.** Their shaders
are byte-identical on both rows (the memory dump is the same): neither reads a
constant in a way the one-constant limit changes. The hwdb row of this part
has `SH_NO_ONECONST_LIMIT` set (read off the row's feature table, not
measured on the board), and the first measurement that moves is mode 25, a
`vec4` array read at a uniform index: three words of the first shader
instructions differ (`+0x08` 40aa0050 -> 60000850, `+0x0c` 00000002 ->
00000007, `+0x18` 80040040 -> 80020040) and `SH_HALTI5_UNIFORMS[0]` gets
`0x2 0xffef7000 0 0` where the GC7000UL gets `0x2 0x10 0xffef7000 0`. Read
these shader words before trusting a diff of an indexed-uniform shader across
the two parts.

**Q11, the wide line (mode 28, `glLineWidth(4.0)` and one `GL_LINES` draw):**

| | GC7000UL | GCNANOULTRA31 |
| --- | --- | --- |
| `ALIASED_LINE_WIDTH_RANGE` | 1 8192 | **1 1** |
| `GL_LINE_WIDTH` after `glLineWidth(4)` | 4 | 4 |
| `PA_LINE_WIDTH` (0x00a18) | `0x40000000` (2.0) | `0x3f000000` (0.5) |
| `PA_WIDE_LINE_WIDTH0/1` (0x00a38/3c) | `0x40000000` (2.0) | `0x3f000000` (0.5) |

The registers hold half the width. The GC7000UL gets 4.0; the ST row gets
Mesa's clamp to 1.0, because Mesa sets `max_line_width` to 1 when the hwdb has
`WIDELINE_TRIANGLE_EMU` (`etnaviv_screen.c`), which this row has. A client on
Mesa sees a line range of 1..1 on this part, while GL still returns the 4 it
was given. What the part itself does with a wider line is a board question.

`cs_golden.c` mode 28 is golden-only (`cs_ours.c`'s mode list says so).

`gate_st.sh hwdb|golden` runs the two checks of this section against a Mesa
26.2.1 tree (`MESA_ETNA_TREE`, built per "Building it" for `golden`): the hwdb
comparison against `testdata/hwdb-compare-st-6205.txt`, and the signature of
modes 1, 8 and 15 on the two shim rows. Exit 0 pass, 1 mismatch, 2 gap (no
tree: the check was not run), 64 usage. `gate_st_selftest.sh` checks the
script without a tree.

### A tile-status fast clear: mode 29 and `testdata/ts-clear.golden`

The host test of `docs/notes/gc-tiled-targets.md` step 0: what Mesa tells
the GC7000UL when a tiled target with tile status (TS) is fast-cleared, drawn
into and resolved to linear. Mode 29 renders into a 64x64 `GL_RGBA8_OES`
renderbuffer with a 64x64 `GL_DEPTH24_STENCIL8_OES` one, clears all three
buffers in one `glClear` to values no other mode uses (colour 0.25, 0.5, 0.75,
1.0; depth 0.75; stencil 0x5a), draws mode 1's triangle under `GL_LESS`, and
reads the target back with `glReadPixels`. Mesa allocates the target tiled
(super-tiled) with a TS buffer, so the clear touches the TS buffers alone and
the readback is the resolve: the RS copies the tiled surface into a linear
staging buffer with the source's TS bound.

The committed golden is `cs_decode.py`'s print of the mode's
`ETNA_SHIM_CS_DUMP`, nothing added. It reproduces from the tree under
"Building it", byte for byte (one submit of 550 words; the raw dump's
sha256 is `4cca1be2...0ad48e`, the memory dump's `c08bb343...87acf3c`):

```sh
MESA=<mesa-26.2.1, patched>          # "Building it" above
BUILD=<build dir>/mesa-etnaviv-host
cc -O1 -o cs_golden tools/vivante/cs_golden.c -I$MESA/include \
    -L$BUILD/src/egl -L$BUILD/src/mesa/glapi/es2api -lEGL -lGLESv2
rm -f ts-clear.cs ts-clear.mem       # the shim appends to both
LD_LIBRARY_PATH=$BUILD/src/egl:$BUILD/src/mesa/glapi/es2api:$BUILD/src/gallium/targets/dri \
LD_PRELOAD=$BUILD/src/etnaviv/drm-shim/libetnaviv_noop_drm_shim.so \
MESA_LOADER_DRIVER_OVERRIDE=etnaviv ETNA_SHIM_GPU=GC7000UL \
ETNA_SHIM_CS_DUMP=ts-clear.cs ETNA_SHIM_MEM_DUMP=ts-clear.mem \
    ./cs_golden 29
python3 tools/vivante/cs_decode.py ts-clear.cs \
    --regs $MESA/src/etnaviv/hw/state_3d.xml.h \
    --regs2 $MESA/src/etnaviv/hw/state.xml.h \
    | diff tools/vivante/testdata/ts-clear.golden -
```

It prints `renderer: Vivante GC7000 rev 6204` and `drew; gl error 0x0`
and exits 0, and the diff is empty. `MESA_LOADER_DRIVER_OVERRIDE` is needed
on a host with a GPU of its own: without it the EGL loader names the host's
driver (`virtio_gpu: driver missing`) and `eglInitialize` fails. Measured on
2026-11-07 with the build recipe above. The tree compiled 1022 of 1022 steps
with pycparser 2.22 taken from its source checkout, put on the path of the
Python meson runs. Every other mode's two dumps came out byte-identical
with and without this change to `cs_golden.c` (modes 1-5, 7, 8, 10-12,
14-28; 16 exits 3 as before).

What to read in it (`cs_decode.py` names a multi-register `LOAD_STATE` by its
first register; the words after it are the registers that follow, 4 bytes
apart):

| what | registers | words |
| --- | --- | --- |
| the fast clear | two RS fills, colour TS then depth TS | `RS_CONFIG` 0x00004606 (A8R8G8B8 to A8R8G8B8, `DEST_TILED`), `RS_DEST_STRIDE` 0x100, `RS_WINDOW_SIZE` 0x00040010 (16 x 4), `RS_CLEAR_CONTROL` 0x0001ffff (`ENABLED1`, all 16 bits) with fill 0x11111111, `RS_PIPE_DEST_ADDR` the TS buffer (0xffff6000 colour, 0xffff1000 depth) |
| the draw's TS state | `TS_MEM_CONFIG` (0x01654) count 7 | 0x000043c3 (`DEPTH_FAST_CLEAR`, `COLOR_FAST_CLEAR`, `DEPTH_COMPRESSION`, `COLOR_COMPRESSION`, compression format 3, `STENCIL_ENABLE`), then `TS_COLOR_STATUS_BASE` 0xffff6000, `TS_COLOR_SURFACE_BASE` 0xffff7000, `TS_COLOR_CLEAR_VALUE` 0xff4080bf, `TS_DEPTH_STATUS_BASE` 0xffff1000, `TS_DEPTH_SURFACE_BASE` 0xffff2000, `TS_DEPTH_CLEAR_VALUE` 0xbfffff5a; `TS_COLOR_CLEAR_VALUE_EXT` 0xff4080bf |
| the PE's view | `PE_MEM_CONFIG` (0x014bc, the 8th word after `PE_STENCIL_CONFIG_EXT`) | 0x05000000 (`COLOR_TS_MODE`, `DEPTH_TS_MODE`); `PE_COLOR_FORMAT` (0x0142c, 7th word after `PE_DEPTH_STRIDE`) 0x00112f06, super-tiled; `PE_DEPTH_CONFIG` 0x05051111; `RA_EARLY_DEPTH` 0x05000031 |
| the resolve | `TS_MEM_CONFIG` 0x00000382 (colour fast clear and compression only), `TS_COLOR_*` as above, then one RS copy | `RS_CONFIG` 0x20000686 (`SOURCE_TILED`, `SWAP_RB`), `RS_SOURCE_STRIDE` 0xa8000400 (`TILING`, `TS_MODE`, `SUPER_TILED_NEW`, stride 0x400), `RS_DEST_STRIDE` 0x100, `RS_WINDOW_SIZE` 0x00400040 (64 x 64), `RS_CLEAR_CONTROL` 0, source 0xffff7000, dest 0xffeea000 (the linear staging buffer) |

The colour clear value is the A8R8G8B8 word of (0.25, 0.5, 0.75, 1.0), and
the depth one the D24S8 word of 0.75 (0xbfffff) with stencil 0x5a; the TS
fill 0x11111111 is the "cleared" tile-status code, not a colour: Mesa's
`specs.ts_clear_value` for a part with 4 bits a tile and no DEC400
(`etnaviv_screen.c`, `bits_per_tile`; 0x55555555 at 2 bits, 0xffffffff
with DEC400).
Addresses are where the shim's allocator put the buffers; compare slots and
contents, not the numbers.

**There is no `cs_ours` mode 29**, and none is needed yet: this backend has
no tile status (linear targets, clears by quad), so there is nothing on our
side to diff. `cs_ours.c`'s mode list reserves the number as golden-only.
The mode is the reference the tile-status steps of `gc-tiled-targets.md`
are checked against once they exist.

### A TILED target with super-tiled depth and early depth: mode 30 and `testdata/tiled-ez.golden`

The words `docs/notes/gc-tiled-wire.md` W4 (a 32-bit tiled shadow target
resolved by the RS in the same stream) and W5 (early depth into it) test
against. Mode 30 renders into a 64x64 RGBA8 **texture** attached to an FBO
(a shadow is a texture the scene renders into) with a 64x64
`GL_DEPTH24_STENCIL8_OES` renderbuffer, enables `GL_LESS`, draws mode 1's
triangle with no clear, and calls `glReadPixels`, whose resolve is the RS
tiled-to-linear copy. Golden only: this backend has no `cs_ours` mode 30.
The golden is `cs_decode.py`'s print of one submit of 462 words, nothing
added; the raw dump's sha256 is `d4ec4a26...04c2e`, the memory dump's
`a82c4707...d373f`. It reproduces like mode 29's, with `30` for `29`, `tiled-ez`
for `ts-clear`, and the same build (cc line included):

```sh
cc -O1 -o cs_golden tools/vivante/cs_golden.c -I$MESA/include \
    -L$BUILD/src/egl -L$BUILD/src/mesa/glapi/es2api -lEGL -lGLESv2
rm -f tiled-ez.cs tiled-ez.mem
LD_LIBRARY_PATH=$BUILD/src/egl:$BUILD/src/mesa/glapi/es2api:$BUILD/src/gallium/targets/dri \
LD_PRELOAD=$BUILD/src/etnaviv/drm-shim/libetnaviv_noop_drm_shim.so \
MESA_LOADER_DRIVER_OVERRIDE=etnaviv ETNA_SHIM_GPU=GC7000UL \
ETNA_SHIM_CS_DUMP=tiled-ez.cs ETNA_SHIM_MEM_DUMP=tiled-ez.mem \
    ./cs_golden 30
python3 tools/vivante/cs_decode.py tiled-ez.cs \
    --regs $MESA/src/etnaviv/hw/state_3d.xml.h \
    --regs2 $MESA/src/etnaviv/hw/state.xml.h \
    | diff tools/vivante/testdata/tiled-ez.golden -
```

Measured 2026-11-13 on the tree of mode 29 (1022 of 1022 steps; pycparser
2.22 from its `release_v2.22` checkout on `PYTHONPATH`); mode 29 reproduced
`ts-clear.golden` byte for byte with the same binary.

**Mesa does not give an RGBA8 target TILED colour with super-tiled depth.**
On the GC7000UL (`SUPER_TILED`, `SUPERTILED_TEXTURE`) it allocates the colour
texture super-tiled as well, with a tile-status buffer; Mesa keeps a colour
target TILED only for the 128-bit formats (no ES 2.0 render target), or when
`ETNA_MESA_DEBUG=no_supertile` takes the super-tiled depth away too. The
committed golden is the default run. The TILED colour words are the same mode
run with `ETNA_MESA_DEBUG=no_supertile`, and differ from the golden in exactly
these three lines, so the table gives both:

| what | register (address) | default run (golden) | TILED colour (`no_supertile`) |
| --- | --- | --- | --- |
| colour tiling | `PE_COLOR_FORMAT` (0x0142c, the 7th word of the `PE_DEPTH_STRIDE` load at 0x01414) | 0x00112f06: format 6 (A8R8G8B8), components 0xf, `OVERWRITE` 0x10000, `SUPER_TILED` 0x100000, `SUPER_TILED_NEW` 0x2000 | 0x00010f06: the same without the two super-tiled bits |
| depth | `PE_DEPTH_CONFIG` (0x01400) | 0x05051111: `DEPTH_MODE_Z`, `D24S8`, `EARLY_Z` 0x10000, `SUPER_TILED` 0x04000000 | 0x01051111: no `SUPER_TILED` |
| RS source | `RS_SOURCE_STRIDE` (0x0160c) | 0xa8000400: `TILING`, `TS_MODE`, `SUPER_TILED_NEW`, stride 0x400 | 0x20000400: `TS_MODE`, stride 0x400, **no** `TILING` bit |

Words that are the same in both runs, and are what W4 and W5 write:

| what | register (address) | value |
| --- | --- | --- |
| early depth | `RA_EARLY_DEPTH` (0x00e08) | 0x05000031 |
| depth | `PE_DEPTH_CONFIG` (0x01400, count 4), then `PE_DEPTH_STRIDE` 0x100 | `EARLY_Z` set (above); near/far 0x3f800000, 0x4b7fffff |
| colour | `PE_COLOR_STRIDE` (0x01434) | 0x100 (the 64-pixel row of 4 bytes; the RS source stride is that times 4 in the TILED forms) |
| single buffer | `PE_LOGIC_OP` (0x014a4, the 2nd word of the `PE_STENCIL_CONFIG_EXT` load at 0x014a0) | 0x000e426c: `SINGLE_BUFFER` field (0x300) = 2 |
| PE memory | `PE_MEM_CONFIG` (0x014bc, the 8th word of that load) | 0x05000000 (`COLOR_TS_MODE`, `DEPTH_TS_MODE`) |
| tile status | `TS_MEM_CONFIG` (0x01654, count 7) | 0x000043c0 and the TS bases (0xfffec000, 0xfffed000 colour; 0xffff1000, 0xffff2000 depth). Nothing is cleared, so the fast-clear bits are armed (mode 29: 0x43c3) but no clear value is written |
| the resolve | `TS_MEM_CONFIG` | 0x00000000 (the resolve reads the surface, not the TS) |
| the resolve | `RS_CONFIG` (0x01604) | 0x20000686 (`SOURCE_TILED`, `SWAP_RB`, A8R8G8B8 to A8R8G8B8, destination linear) |
| the resolve | `RS_DEST_STRIDE` (0x01614) | 0x100 |
| the resolve | `RS_PIPE_SOURCE_ADDR[0]` / `RS_PIPE_DEST_ADDR[0]` (0x016c0 / 0x016e0) | the colour surface (0xfffed000) / the linear staging buffer (0xffee5000) |
| the resolve | `RS_WINDOW_SIZE` (0x01620) | 0x00400040 (64 x 64) |
| the resolve | `RS_CLEAR_CONTROL` (0x0163c, count 5) | 0 |
| the resolve | `RS_SINGLE_BUFFER` (0x016b8) | 1 before the kick, 0 after it |
| the resolve | `RS_KICKER` (0x01600) | 0xbeebbeeb |

The order in the stream is the point for W4: the draw, `GL_FLUSH_CACHE` 3,
a semaphore and stall, `TS_MEM_CONFIG` 0, then the RS block above, ending in
the kick and `RS_SINGLE_BUFFER` 0. The draw's state is `PE_DEPTH_CONFIG`
through `TS_MEM_CONFIG` (lines 54-62 of the golden). Addresses are where the
shim's allocator put the buffers; compare slots and contents, not the numbers.
The `no_supertile` run is the evidence for the TILED forms; only the default
run is committed. What the GC7000UL RS does with a TILED source is not
something a host run can show.

### What the patch does, and why each piece is needed

Four changes, all to the shim, none to the driver (`GC7000UL` is the first
of the entries that carry ids; `GCNANOULTRA31` is the second, see "Another
part: the ST row"):

1. **A `GC7000UL` entry** with the board's ids (model `0x7000`, revision
   `0x6204`, product `0x70003`) and *no* feature words, plus DRM version 1.4
   for that entry only. With 1.4 the driver asks for the product id and looks
   the core up in NXP's own feature database, which is the authority on what
   this part is -- and says **halti 5**, 320 constants. The stock `GC7000L`
   entry is a different part (revision `0x6214`) with hand-copied feature
   words. Only an entry that carries the ids may claim 1.4: the others' param
   arrays stop at `SOFTPIN_START_ADDR` and reading past it is out of bounds.
2. **`GET_PARAM` fails for any core but 0.** The winsys walks core indices
   looking for both a 3D core and an NPU, stopping when a query fails. A shim
   that answers every index alike is walked for ever.
3. **The perfmon queries fail** (with `errno` set -- libdrm retries `EINTR`
   and `EAGAIN`, so a bare -1 is a spin, not a refusal). They are walked until
   one fails, and the stock no-op succeeds for ever.
4. **`GEM_SUBMIT` writes the stream out** to `$ETNA_SHIM_CS_DUMP` instead of
   discarding it, and every buffer the submit references to
   `$ETNA_SHIM_MEM_DUMP`. Those two files are the golden stream and the memory
   it points at. The bytes come out of the shim's memfd at each buffer's own
   device address rather than out of a mapping, because the driver is free to
   have unmapped it by the time it submits.

There is a fifth change outside the etnaviv shim, in `src/drm-shim/drm_shim.c`:
the shim re-resolves two dozen libc symbols with `dlsym(RTLD_NEXT, ...)` on
*every* intercepted call. That is cheap for shader-db; against a whole
EGL/gallium stack the symbol tables are large enough that it never finishes.
Resolving them once fixes it.
