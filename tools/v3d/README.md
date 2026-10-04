# V3D 4.2 (Raspberry Pi 4) reference instruments

The product path for this GPU is our own online compiler (`src/shaderc/online/v3d/`);
there is no offline table. Everything here is an instrument: `v3d_shc` is the Mesa
reference our compiler is compared with on the same source, and the checks below
vouch for the driver on the host.

`v3d_shc.c` is a host tool built inside a Mesa tree (26.2.1), like
`tools/vivante/etna_shc.c`. It takes one GL program as two OpenGL SPIR-V modules
and runs Mesa's own v3d compiler three times, as the gallium v3d driver does for
a draw: the fragment shader, the vertex shader keyed by the fragment shader's
inputs, and the coordinate shader (the same vertex shader for binning). No
device is needed.

A program, not a stage, is the unit here: the vertex shader writes its varyings
in the order of the fragment shader's `input_slots`, so the vertex shader's
code depends on the fragment shader it is linked with.

Set up once (the tree is not part of this repository):

```sh
cd <mesa-26.2.1>
cp <this dir>/v3d_shc.c src/broadcom/
patch -p1 < <this dir>/mesa-v3d_shc.patch   # adds the meson target
meson setup build-v3d -Dgallium-drivers=v3d -Dvulkan-drivers= -Dplatforms= \
    -Dglx=disabled -Degl=disabled -Dgbm=disabled -Dllvm=disabled -Dtools=
ninja -C build-v3d src/broadcom/v3d_shc
```

Run it on SPIR-V made the way `tools/vivante/glsl2vivante.py` makes it
(`glslangValidator -G`):

```sh
v3d_shc vs.spv fs.spv [--vpm-size BYTES] > out.json 2> disasm.txt
```

The JSON has the QPU code of `fs`, `vs` and `cs` (64-bit words), each stage's
uniform stream description (`contents` is Mesa's `enum quniform_contents`),
the VPM sizes for the shader state record, the fragment shader's input slots
and flat/noperspective/centroid flags, and the uniform and attribute names.
The disassembly goes to stderr.

Before the v3d compiler sees a shader, the tool runs the passes Mesa's GL
state tracker runs for a GL_ARB_gl_spirv program, in the same order, with the
v3d screen's caps (each pass is cited by Mesa file:line in `v3d_shc.c`):
`_mesa_spirv_to_nir` (glspirv.c), `gl_nir_link_spirv` with
`nir_lower_io_passes` and `nir_opt_varyings_bulk` (gl_nir_linker.c),
`st_link_glsl_to_nir` (st_glsl_to_nir.cpp), and the shader variant st/mesa
creates at the first draw (st_program.c, `st_finalize_nir`,
`nir_unlower_io_to_vars`). What this means for the runtime:

- The two stages are linked and optimised together. A varying the FS does not
  read, or one the VS writes as a constant, can be removed, so the VS and FS
  of one pair belong together; a VS attribute that only fed a removed varying
  is not fetched.
- `attributes`: per attribute, its GL `location` (the SPIR-V Location) and
  `records`, one entry per slot (a `matN` has N): the index of the vertex
  attribute record that slot is fetched through (the compiler's
  `vattr_sizes` index), or `null` when the shader does not read that slot.
- `uniform_names`: per uniform, its `stage` (the stage whose storage holds it
  after the passes above, which can be the VS for a uniform the source reads
  in the FS: `nir_opt_varyings` moves work across the varyings) and its place
  in that storage as a vec4 `slot` and `vec4s`. A QUNIFORM_UNIFORM word is
  `slot * 4 + component`. A vector can be split into one entry per component,
  each at its own slot (es100/utri: `u_scale` at slots 1..4, component k read
  at slot 1 + k), so a name can appear more than once. YakoGL's own compiler
  (`src/shaderc/online/v3d/chain.rs`) writes `word` and `words` instead: the
  IR's dword offset and size, which its QUNIFORM_UNIFORM names directly.
  `tools/v3d/v3d_prog_header.py` reads both.
- `uniform_names` can hold state uniforms with a `state` field. v3d's pixel
  origin is upper left, GL's is lower left, so a `gl_FragCoord` read uses
  `gl_FbWposYTransform` (`FB_WPOS_Y_TRANSFORM`) and a `gl_PointCoord` read
  uses `gl_PntcYTransform` (`FB_PNTC_Y_TRANSFORM`). The runtime writes them
  as Mesa's prog_statevars.c:736-763 does: WPOS is `(1, 0, -1, height)`
  when the framebuffer is not flipped and `(-1, height, 1, 0)` when it is;
  PNTC is `(-1, 1, 0, 0)` when the point origin is lower left XOR the
  framebuffer is flipped, else `(1, 0, 0, 0)`.
- The FS is compiled for a triangle draw (`is_points` false in the FS key).
  A shader that reads `gl_PointCoord` compiles, but that variant reads a
  register nothing writes; a point draw needs a points variant, which the
  tool does not build yet.

`gpu.vpm_size_source` is `assumed` until the VPM size read from the board
(`IDENT1.VPM_SIZE * 8192`) is passed with `--vpm-size`; the compiler uses it for
the VPM configuration, so a table built on the assumption is not final.

On the lab host the tree is `~/pvr-lab/mesa-build/mesa-26.2.1` and the tool is
built there (`build-v3d/src/broadcom/v3d_shc`).

## Host checks for the driver (driver/src/v3d)

Five programs check the V3D driver on the host, before any board time. Each
exits non-zero on any failure and when the number of cases it ran differs
from the number it planned, and each was run once against a deliberately
broken copy of the code to show that it can fail.

`tools/check.sh` runs `tex_check`, `cl_golden` and `job_check` against Mesa
26.2.1 and requires each one's own "run (planned)" line. Nothing outside the
repository is needed: `tools/v3d/mesa_ref.sh` builds, from the sources
vendored unmodified in `third_party/mesa-26.2.1`, the packers Mesa generates,
its CLIF decoder, the part of `src/util` that needs, and its format table,
into `build/v3d-check/mesa/` (`tools/v3d/mesa-stub/` holds the two gallium
driver headers and libdrm's that the table and the shared headers include and
nothing here uses). `MESA_V3D_TREE`, a Mesa tree with `build-v3d` (the
lab tree above), compares against that tree instead; set to something that is
not one, the rows fail.

- `cl_golden.c` (with `cl_golden_mesa.c`): the packets are Mesa's own
  generated packers since 2026-10-01 (`driver/src/v3d/generated/v3d_packet_v42_pack.h`,
  made by the vendored `gen_pack_header.py` from the vendored
  `v3d_packet.xml`; `tools/check.sh` regenerates it and requires the
  committed bytes). cl_golden checks what `v3d_cl.h` adds: for each of the 60
  packets and records the driver writes (`v3d_cl_packets.h`), the struct
  against Mesa's build, and 20,002 value sets (Mesa's unpacker applied to
  all-zero, all-ones and random bytes) never refused and packed to the
  bytes of Mesa's own build; then the refusals (a field too wide, a signed
  field out of range, a misaligned address, a minus-one field at 0, a
  fixed-point or f187 value off its grid, a full list) and `v3d_cl_emit`.
  `1200195 cases run (planned 1200195), 0 failed`. Needs `build-v3d`.
- `frame_check.c`: the clear frame of `driver/src/v3d/v3d_frame.c` decoded by
  Mesa's own CLE decoder (`clif_dump`), which follows the RCL into the
  per-tile list; the packet census must be Mesa's for the same frame. Needs
  `build-v3d` and `ninja -C build-v3d src/broadcom/cle/libbroadcom_cle.a`.
  `--triangle` (built with `-DFRAME_CHECK_TRIANGLE` and the generated
  `tri_prog.h` on the include path, run with `FRAME_CHECK_JSON=<the JSON>`)
  decodes the M5 clear-then-triangle frame: clif follows `GL_SHADER_STATE`
  into the shader state record and the attribute records. It checks the
  packet census (and that no decoded packet is outside it), every decoded
  field of the record and the attribute records against the value Mesa's
  `emit_shader_state_record` derives from v3d_shc's JSON (read by
  frame_check itself, not through the header), the code buffers the record
  points at word for word against the JSON, the uniform streams against
  `v3d_write_uniforms`' rule per `quniform_contents`, and the default
  attribute values. A field the JSON lacks fails the check unless the header
  was generated with an `--assume` for it; the result line counts such reads.
- `hw_mock_test.c`: `driver/src/v3d/v3d_hw.c` against a software model of the
  V3D, PM and ASB registers (`hostshim/` stands in for Zephyr): the order of
  the power-on, MMU and submit writes, and that an unacknowledged bridge, a
  binner out of memory, an MMU fault and a timeout are four different
  results; and the TFU job (`v3d_tfu_run`): the register order of Linux's
  v3d_tfu_job_run, the TFUC wait with CVTCT as a second witness, a stale
  TFUC cleared first, busy, fault, timeout and a missing count apart. It
  proves the sequence, not the silicon. A check.sh row since the TFU.
- `job_check.c`: the job builder `driver/src/v3d/v3d_job.c` (M6): identity
  with the M4/M5 frames, the rollback of a draw that does not fit, a
  three-draw job and the clear quad (M6 step B) through Mesa's CLE decoder.
  Needs `build-v3d`.
- `tex_check.c`: the textures of `driver/src/v3d/v3d_tex.c` (M6 textures).
  Since 2026-10-01 the driver tiles through Mesa's own `v3d_tiling.c`
  (vendored, `driver/src/v3d/v3d_tiling_mesa.c`) and packs the records with
  Mesa's generated packers, so the tiling and record parts below check what
  `v3d_tex.c` hands Mesa (the slice, its offset and padded height, the box);
  the texel addresses of `tools/v3d/tex_pixel_offset.h` remain an
  independent reference for tfu_case, tex_tiling_bench and tex_upload_check.
  It checks
  the level layout against numbers derived by hand from Mesa's
  `v3d_setup_slices` (static, so it cannot be linked), the texel tiling byte
  for byte against Mesa's `v3d_store_tiled_image` (from `libv3d_neon.a`) for
  every level of 13 sizes at every cpp, and the texture and sampler state
  records against `V3D42_*_pack()`. Since the femtovg work it also checks:
  sub-image stores (`v3d_tex_store_box`, what `glTexSubImage2D` needs; femtovg
  updates its 256x1 gradient texture that way) against the same Mesa
  function with a `pipe_box`, 2990 boxes on every level of 9 sizes at every
  cpp, the whole image compared after every box, plus 8 refusals; the source
  formats against Mesa's own format table (`v3dx_format_table.c`, compiled
  by the script with Mesa's flags; `tex_check_mesa_fmt.c` reads it): RGBA8
  and BGRA8 equal Mesa's rows, and RGB8, L8, A8, L8A8, RGB565, RGBA4 and
  RGB5_A1 are refused by name; every GL ES 2.0 sampler (6 min x 2 mag
  filters x 3 x 3 wraps, and femtovg's four) against `V3D42_SAMPLER_STATE_pack`
  of what `v3d_upload_sampler_state_variant` fills; five negative
  controls that must be detected in every run; the packers' refusals (a type
  no supported format has, any field that does not fit: nothing is masked);
  and the TMU P0/P1 uniform words (`v3d_tex_p0/p1`) against
  `V3D42_TMU_CONFIG_PARAMETER_0/1_pack`; and layered images (cube maps):
  the layer stride, size and offsets against numbers derived by hand from
  Mesa's cube_map_stride rule, all 6 faces x every level tiled against
  Mesa's store at Mesa's offsets, and the depth-6 record against
  `V3D42_TEXTURE_SHADER_STATE_pack`. Measured 2026-09-24:
  `351 checks run (planned 351), 0 failed`. Removing the level-1 page
  alignment, `ub_pad` or the UIF XOR each fails it, and so do seven mutants of
  `v3d_tex.c` (listed with their counts in the commit that added them).
  Texel data comes from xorshift32. Two earlier generators were periodic
  (`i * 131 + 1`, period 256 bytes; the low bits of an LCG, period 64 KiB),
  and UIF XOR moves data by a multiple of both, so XOR errors went unseen
  at 512x512 and 256x256.
  `tools/v3d/tex_check.sh` builds and runs it (against the vendored Mesa
  sources; `MESA_V3D_TREE` optionally names a built Mesa tree instead;
  output in `build/v3d-check/`), and `tools/check.sh` runs that script.
- `tex_filter_ref.h` and `tex_linear_case.h`: the LINEAR board test (dEQP's
  filters as the reference, the scene, the four draws, the verdict and the
  controls), header only, for the samples; `tex_filter_ref_check.c` checks
  them (26 checks). No Mesa tree needed.
- `tex_narrow_case.h`: gl_test 135 (texnarrow), L8, A8 and LA8 sampled
  narrow (R8 / RG8 with a swizzle): the draws, the hashed texels, the
  controls and the verdict; `tex_filter_ref_check.c` part 9 checks it.
- `tex_tiling_bench.h`: times `v3d_tex_store_level` on the board, per size,
  in the caller's memory (it must be the OSAL's non-cacheable `gpu_alloc`),
  and checks every tiled texel; `tex_tiling_bench_check.c` checks the
  harness on the host (19 checks; its timings judge nothing).
- `tfu_case.h`: the TFU probe of `v3d_gl_test` case 140 (tfuprobe): the job
  fields packed as Mesa's v3dx_tfu.c does for V3D 4.2, the formats, sizes
  and source variants, and the texel-by-texel compare with
  `v3d_tex_store_level`; `tfu_case_check.c` checks it on the host (the sizes
  reach every tiling, every field against its source, one job as Mesa's
  literals, the refusals, and a compare that finds each kind of error).
- `tex_upload_check.c`: tile at upload. The copy the backend makes at the
  upload from the caller's rows (`v3d_tex_store_box` at the GL core's
  unpack stride) and the copy it makes at a draw from the GL core's storage
  (`v3d_tex_store_level` at `tex_pitch`) are the same bytes, whole image
  and padding: 19 sizes x UNPACK_ALIGNMENT 1/2/4/8 x UNPACK_ROW_LENGTH
  0/w/w+5 = 228 cases, plus two controls that must differ (72 of 72 each)
  and the short-stride refusal: `231 checks run (planned 231), 0 failed`.
  It mirrors two rules from the Rust (`tex_pitch`, `src_row_bytes`,
  cited by file:line in its header); the backend's own tests pin those.

`v3d_prog_header.py` turns one v3d_shc JSON into the C header the frame
builder takes (`v3d_frame_build_triangle`): the QPU code, the uniform
streams and a function per stage that fills `struct v3d_fs_prog` /
`v3d_vs_prog`. A prog_data field the frame builder reads and the JSON lacks
is an error, not a default; `--assume stage.key=value` gets past it, and the
header records every assumption in `<NAME>_ASSUMED` for the image to print.
It also refuses what the builder cannot write (spills, non-zero varying
flags, a uniform `contents` the builder does not support, a points or lines
variant, a VS that writes the point size, a shader with `tmu_dirty_rcl`).
`driver/samples/zephyr/v3d_triangle/CMakeLists.txt` runs it at build time
with no `--assume`: every field comes from the JSON.

The build commands are in each file's header comment. None of them is a
statement about the GPU: the byte reference for a whole frame is Mesa's
`V3D_DEBUG=clif` dump of the same clear (and, for M5, the same triangle) on
the board.

## Primitive variants (`--prim`)

Mesa compiles the fragment shader for the primitive being drawn
(`v3d_update_compiled_fs`: `is_points`, `is_lines`) and gives a points draw the
vertex shader's point size (`v3d_update_compiled_vs`: `per_vertex_point_size`,
set here when the VS writes `gl_PointSize`). `v3d_shc ... --prim points` builds
that variant; the default is `triangles`, and `gpu.prim` in the JSON names it.
Measured 2026-09-24: `points.vert` + `es100/pointcoord.frag` is FS 6 / VS 21 / CS 22
for triangles and FS 10 / VS 23 / CS 24 for points (the FS gains the two
`ldvary` of the point coordinate); `color_uv` is 12 / 31 / 22 in all three.
The runtime picks the variant at draw time from the primitive.

## Sampler indices

A sampler's texture index is the SPIR-V binding the GLSL translator gave it
(`uniform_names[].unit`), not an index `gl_nir_lower_samplers` derives from
uniform storage. The runtime binds a texture unit to that index through
`glUniform1i` on the sampler uniform, so the table carries the binding and the
name, and the mapping is made at draw time.

## The QPU instruction set in the online compiler (src/shaderc/online/v3d)

`src/shaderc/online/v3d/` is the V3D 4.2 QPU instruction set as a Rust module:
a decoder, an encoder, program helpers and a listing in Mesa's text format. It
is the base of the V3D's only product path, our own online compiler.
Instruction selection is not written yet. Mesa's v3d compiler (`v3d_shc`) is
only the reference we compare against. The module is compiled wherever
`online` is, like `vivante` and `powervr`, and its tests run in every
`shaderc-online` row of `tools/check.sh`. The tables are transcribed from
Mesa 26.2.1 `src/broadcom/qpu/qpu_pack.c` (MIT, ver 42 only). No other
source was used.

The contract is **correct or refused**. The encoder decodes every word it
writes, and refuses an instruction whose word would read back as anything
else. So "nothing encodes silently wrong" rests on the decoder, and the
decoder is checked against Mesa on every combination of every field group,
not only on the words Mesa's compiler emits.

### The corpus

`qpu_corpus.txt` is every distinct 64-bit word in the fs, vs and cs code of
every program pair that `glsl2v3d.py` compiles. Regenerate it with:

```sh
tools/v3d/qpu_corpus.py                  # runs glsl2v3d.py itself, or
tools/v3d/glsl2v3d.py --json DIR/pairs.json --keep DIR/keep
tools/v3d/qpu_corpus.py --from DIR       # reuse that run
```

The header holds the counts, and `roundtrip.rs` pins them. A corpus that
grows fails the pin, and the pin is changed on purpose. As generated
(a87ed25c, Mesa 26.2.1): 1914 of 2717 pairs compile, which gives 1,630,858
words and 37,427 distinct words.

### The Mesa reference: qpu_ref.c

`qpu_ref.c` prints Mesa's answer for each word on stdin: whether it unpacks,
its re-pack, and its text. Its header has the build command. It compiles
Mesa's three qpu sources itself with `-DNDEBUG`, because build-v3d keeps
assertions on and Mesa asserts on some words it accepts. Mesa's
`v3d_qpu_magic_waddr_name` reads past the end of its table for a magic
address 56..63 (garbage or a segfault, seen here). `qpu_ref` answers those
words `OOB` instead of calling it.

### The tests

Run them with `cargo test --no-default-features --features host-test,shaderc-online -- v3d::`.

- `golden`: the 45 ver 42 rows of Mesa's `tests/qpu_disasm.c`. Each row
  decodes to Mesa's exact text and encodes back to the same word. The 7
  FADD/FADDNF/FMAX rows also encode back with their operands swapped.
- `roundtrip`: every corpus word decodes and encodes back bit for bit
  (37,427 processed, 0 skipped, 0 differing). Each value the encoder writes
  that the model does not carry is tested against the corpus, with its
  population. The pins name what the corpus cannot measure: 25 operations
  for the unread-operand mux, "bdu without ub" and "offset on an lri/regfile
  branch". A sweep of 1.3 M random words checks that every word the decoder
  accepts encodes back, and that each of the 18 refusal causes fires. There
  are two negative controls: a corrupted FMUL entry fails the round trip,
  and an SHL/SHR name swap passes the round trip but fails the golden text.
- `mesa_ref`: 590,080 words that enumerate every field group (the add half,
  the mul half, signal x flags x raddr_b, every write address, every branch
  field). They are compared with Mesa's answer under a short, named rule for
  the words we refuse on purpose. The gate pins the digest of Mesa's lines
  and recomputes it from our decoder alone. Renaming one operation that the
  corpus never uses (UMAX) fails it. The word-by-word comparison is
  `#[ignore]` because it needs the binary:
  `V3D_QPU_REF=<qpu_ref> cargo test ... mesa_ref -- --ignored --nocapture`.
  Measured 2026-09-24: 0 of 590,080 differ. Classes: 238,665 text, 768
  FTOI* L/H text (decoded, refused by the encoder), 12 IID, 286,632 Mesa
  refuses, 44,724 Mesa lossy, 766 small immediate past the table, 48 LDVPMP
  with MA, 3,105 unnamed magic address, 15,360 bdu 4..7. The gate pins the
  768 as its own count.
- `listing_matches_mesa_on_the_corpus` is `#[ignore]` because it needs the
  `--keep` directory (`V3D_KEEP_DIR=DIR/keep`). Measured 2026-09-24:
  1,630,858 lines, 0 differ.

### Where we differ from Mesa

Mesa's model cannot hold some bit patterns its unpack accepts, and Mesa's
pack(unpack(w)) is not w for them. Mesa also accepts some words it asserts
against or cannot name. This decoder refuses each of these with its own
`DecodeError`, and the encoder refuses the matching instructions:
- STVPM with MA, and LDVPMP with MA;
- the FXCD/FYCD pack variants;
- a small immediate past the table;
- a magic write, or a magic signal address, that Mesa's table does not name
  (25..31, 47..54, 56..63);
- branch condition 1, and bdu 4..7;
- a branch's unused bits, bdu without ub, unused raddr_a, or an unused
  offset.

None of them is in the corpus.

One name differs from Mesa on purpose: Mesa prints the 4.2 IID word as
`vdwwt`, because VDWWT comes first in its table. The model names it `Iid`.

**The encoder refuses FTOIN/FTOIZ/FTOUZ/FTOC with the L or H unpack
(`EncodeError::UnpackUnmeasuredOnSilicon`) until a board run settles
them.** The decoder still reads those words as Mesa's decoder does, so it
reads what exists. They are the one named exception to "every word the
decoder accepts encodes back". Mesa disagrees with itself here:
- Its decoder reads opcodes 249/250 as L and 253/254 as H
  (qpu_pack.c:1062-1065, 1150-1160).
- Its encoder ORs the unpack into opcode 245/246, whose bit 2 is already set
  (qpu_pack.c:1795). So it writes 253/254 for both L and H.
- Its compiler asks for L on every 16-bit source (nir_to_vir.c:1459-1467).
So on the board 253/254 runs where Mesa means L, and nothing measured says
which half the hardware reads for either word. The corpus has 178 distinct
words with these operations, all NONE (245/246). A board run that sends an
f16 value with different halves through 249/250 and 253/254 would settle it;
`roundtrip::ftoi_unpacks_decode_but_are_refused_until_measured` pins the
refusal until then. The ES 2.0 compiler never needs an f16 unpack on these
operations.

## The QPU interpreter, validated on Mesa's code (src/shaderc/online/v3d/sim.rs)

`sim.rs` interprets encoded V3D 4.2 QPU programs through our decoder. It
models one QPU with 16 lanes: the add and mul ALUs, r0..r5 and rf0..rf63, the
flags and conditions, the uniform stream, LDVARY (vary and the C coefficient
in r5), the VPM (LDVPMV_IN, STVPMV), TLB writes with their configuration,
THRSW and the program end, and REL branches. It is test-only (`host-test`,
like `eval.rs`). Anything it does not model stops the run with a reason and
never gives a value.

Every semantic comes from Mesa's compiler (MIT), and the doc comment at the
top of `sim.rs` cites each one. It also names the assumptions (IEEE f32,
round-to-nearest-even f16, the program end when there is no last-THRSW pair)
and what is unmodelled (textures, MULTOP, derivatives, LDUNIFA, TIDX, EIDX,
REVF). Two timing rules were first written from Mesa's validator and then
corrected against Mesa's own code:
- an SFU result in the register file can be read by the next instruction,
  because the hardware stalls (`qpu_schedule.c:779-786`, "sfu-stalls");
- Mesa's "Z from FEP" write moves an undefined register to the TLB
  (`emit_frag_end`), and the hardware ignores that value.

`tests/v3d_sim_mesa.rs` runs Mesa's FS, VS and CS for the real program pairs
of the corpus. It needs no Mesa build, because it reads the checked-in
fixture `tests/fixtures/v3d_mesa_programs.txt` (344 KB: 105 pairs, code
words, uniform streams and prog_data). Regenerate the fixture with:

```sh
tools/v3d/sim_fixture.py      # needs v3d_shc, glslangValidator, spirv-dis
```

The IR evaluator runs the same GLSL on the same inputs.

- **One program at a time.** The IR's FS reads what the IR's VS wrote, by
  name. Mesa's FS reads what Mesa's VS wrote: VPM row 4 + i is FS input i.
  Mesa's link step repacks and folds varyings, so a SPIR-V location does not
  name a Mesa input slot.
- **What is compared.** The FS's TLB colour (as f16) and its discards. The
  VS's scaled x and y, Zs and 1/Wc. The CS's clip position and scaled x and
  y. Comparisons are bit for bit, with a stated tolerance only where the
  program ran an SFU op.
- **Result at a0a485d6, measured 2026-09-24:** 105 pairs, 70 tried: 49 agree,
  0 disagree, 21 unmodelled. 35 were not tried (31 are desktop GLSL, which
  our front end refuses). 10,968 values were compared.
- **Negative controls.** In the test, 81 mutants of Mesa's code
  (FADD -> FSUB, or FMUL reading `.l`): 77 are caught, and the 4 survivors
  are named and explained. By hand, LDVARY's r5 made to land one instruction
  early gives 23 disagreements.
- `V3D_SIM_PAIR=<substring>` runs only the matching pairs and prints their IR
  declarations, Mesa's listing and the uniform stream.
- **The three-way check** is `our_programs_agree_with_the_ir_and_with_mesa`,
  in the same file. The fixture lists all 249 real pairs (`real` lines).
  For each pair our compiler (`v3d::chain::compile_program`, Target
  `vpm_size` 65536) compiles, the test compares:
  - (a) the interpreter on our FS/VS/CS words, with our uniform stream;
  - (b) the IR evaluator;
  - (c) the interpreter on Mesa's program, when the pair is in the fixture.

  (a) is compared with (b), and with (c) on the colour, the discards and
  the position rows. The varying rows are not compared, because their order
  is each compiler's. Refusals are counted by NotYet name, as coverage.
  Measured 2026-09-24 at a0a485d6: 249 real pairs plus 1 extra
  (passthrough + interp_color), and 9 compile. (a) vs (b): 6 agree, 0
  disagree, and 3 matrix-attribute pairs are not tried. (a) vs (c): 5 agree.
  12 of 12 mutants of our code are caught.
