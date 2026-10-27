<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the built-in inputs gl_VertexID, gl_InstanceID, gl_FrontFacing, gl_PointCoord

Study of the nine cases of the 2026-10-23 sweep
(`0dce654a8:tools/mali/board-ref/2026-10-23-s6-discard/ours/03-panthor-gltest--sweep-1.log`)
that are refused with "Valhall: a built-in input (gl_FragCoord,
gl_FrontFacing, gl_PointCoord, gl_VertexID, gl_InstanceID) is not lowered yet"
(`MaliError::SpecialInput`, #1384 S6). Host-only: Mesa 26.2.1's programs on
drm-shim, nothing lowered here. Items: study-builtin of plan #153.

## How the references were made

Mesa 26.2.1 is built at `$CONNECTED_DEPS_DIR/mesa-shim` (the `build` form of
`tools/mali/drm_shim_ref.sh`, per `tools/mali/README-pan_shc.md`; the tree is
`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`). Per distinct shader pair, from the
repository root:

```
tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim tools/shaders/es300/vid.vert   tools/shaders/es300/vcolor.frag /tmp/builtin_43
tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim tools/shaders/es300/vid.vert   tools/shaders/es300/dynidx.frag /tmp/builtin_45
tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim tools/shaders/es300/inst.vert  tools/shaders/es300/vcolor.frag /tmp/builtin_47
tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim tools/shaders/es300/vid.vert   tools/shaders/es300/facing.frag  /tmp/builtin_49
tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim tools/shaders/es300/vid.vert   tools/shaders/es300/uniloop.frag /tmp/builtin_98 --uniform-values
python3 -I tools/mali/pan_ref_diff.py extract /tmp/builtin_<n>.trace --draw > tools/mali/testdata/builtin_<n>.expect.json
```

`gl_ref.py` asks for an ES 2 EGL context and Mesa gives it its ES 3.x level, so
`#version 300 es` compiles (glGetError 0). Limits of this reference: one
non-instanced `glDrawArrays` of 6 vertices; `gl_ref.py` binds no ES 3 `in`
attribute (it parses `attribute` only, "attributes (none)"), so the
attribute-fetch descriptor words are not the case's; the shader programs are.
An instanced reference needs an `--instances N` option in `gl_ref.py`
(not done: outside this item); the instance programs below are nevertheless the
ones Mesa links for any instance count.

`tools/mali/testdata/builtin_<n>.{vert,frag,expect.json}` (the shaders are
copies of `tools/shaders/es300/`; the JSON is the `extract --draw` format of
`s4_ffma.expect.json`, words, disassembly, `work_reg_count`, `preload`, FAU).
They are not in any gate row yet (a row would need pan_shc output to be
byte-equal, which is what the lowering will add).

| file | shaders | cases |
|---|---|---|
| builtin_43 | vid.vert + vcolor.frag | 43, 44, 57 |
| builtin_45 | vid.vert + dynidx.frag | 45 |
| builtin_47 | inst.vert + vcolor.frag | 47, 51, 141 |
| builtin_49 | vid.vert + facing.frag | 49 |
| builtin_98 | vid.vert + uniloop.frag | 98 |

The cases' bodies are in `driver/tools/pvrlab/gl_test.c` (the board-ref
`cases.c` of 2026-10-23-s6-discard holds only case 213 discloop; the shaders
are generated into `gl_test_shaders.h` from `tools/shaders/es300/`, checked by
`gen_gl_test_shaders.py --check`). Line numbers below are `gl_test.c`'s.

## The cases and their shaders

| case | name | body | shaders | built-ins read |
|---|---|---|---|---|
| 43 | es3vid | gl_test.c:6503 (`case_es3vid`) | vid.vert, vcolor.frag | gl_VertexID, `glDrawArrays` |
| 57 | es3vid_indexed | :6539 (comment :6530) | same | gl_VertexID, `glDrawElements` u16 indices 5,4,3,2,1,0 |
| 44 | es3hdr | :6759 | vid.vert, vcolor.frag (colour pass) + cube.vert, hdrsample.frag (works already) | gl_VertexID; also an RGBA16F target |
| 45 | es3dyn | :6903 | vid.vert, dynidx.frag | gl_VertexID; the fragment shader indexes `uniform vec4 cols[8]` by `idx + i` in a loop bounded by `uniform int n` |
| 49 | es3facing | :7368 | vid.vert, facing.frag | gl_VertexID (VS) and gl_FrontFacing (FS) |
| 98 | uniloop | :21265 (comment :21243) | vid.vert, uniloop.frag | gl_VertexID; the fragment shader is an int loop bounded by `uniform float count` |
| 47 | es3inst | :6999 | inst.vert, vcolor.frag | gl_InstanceID, 4 instances, per-instance attributes `offs`, `icol` |
| 141 | es3instdiv | :7090 | inst.vert, vcolor.frag | gl_InstanceID, instance attributes with a divisor past 1 |
| 51 | es3heavy | :7438 (comment :7430) | inst.vert, vcolor.frag | gl_InstanceID, a large instance/primitive volume |

Shader sources (`tools/shaders/es300/`):

- `vid.vert`: `int i = gl_VertexID % 4; v_col = vec4(i == 0 || i == 3 ? 1.0 : 0.0, i == 1 || i == 3 ? 1.0 : 0.0, i == 2 || i == 3 ? 1.0 : 0.0, 1.0);`
  one `vec4 pos` attribute at location 0, one `vec4 v_col` varying.
- `inst.vert`: `gl_Position = vec4(pos.xy * 0.25 + offs, 0.0, 1.0); v_col = gl_InstanceID == 3 ? icol * 0.5 : icol;`
  attributes `pos` (0), `offs` (1), `icol` (2).
- `facing.frag`: `fragColor = gl_FrontFacing ? vec4(0,1,0,1) : vec4(1,0,0,1);` (mediump).

## The refusal sites (src/shaderc/online/mali)

The message is `MaliError::SpecialInput` (mod.rs:198, text mod.rs:402-404). It is
returned by:

- the vertex lowering, `src/shaderc/online/mali/nir.rs:1358`: `IrOp::LoadInput`
  of any slot but `IoSlot::Attrib` (so `IoSlot::VertexId`, `IoSlot::InstanceId`,
  `ir.rs:131-134`). Cases 43, 44, 45, 49, 57, 98 (vid.vert) and 47, 51, 141
  (inst.vert) stop here.
- the fragment lowering, `src/shaderc/online/mali/nir_fs.rs:1393`: `LoadInput`
  of `IoSlot::FrontFacing` (the arms above it take `PointCoord`, nir_fs.rs:1383,
  and a highp `FragCoord`, :1391). Case 49's `facing.frag` stops here once its
  vertex shader passes.
- `nir_fs.rs:1766`: gl_FragCoord z/w reaching a root (not one of the nine).

gl_PointCoord is already lowered and PASSes on the board (cases 83
pointcoord and 105 pointcoordvary): `LD_VAR_SPECIAL.point`
(`nir_fs.rs:1383`, `bir.rs:394`; Mesa: `pan_nir_lower_var_special.c:24`,
`BI_VARYING_NAME_POINT`, read through `bifrost_compile.c:1420`
`bi_emit_load_var_special_pan`). It needs nothing here.

## Where Mesa reads each built-in

Register numbers: `bi_preload_reg` (`src/panfrost/compiler/bifrost/compiler.h`
~1180-1215), Valhall (arch >= 9). A program's `preload` mask has bit n for
register n (`0x38 << 56` = r59, r60, r61).

| built-in | Valhall source | Mesa |
|---|---|---|
| gl_VertexID | **r60** of the IDVS vertex (varying and position) shader, a plain `MOV` of the preloaded register. No base or offset is added: on Valhall's malloc-IDVS flow the hardware's r60 already is the vertex number (non-indexed: `first + n`; indexed: the index plus the bias). | `bifrost_compile.c:2039` `load_vertex_id` -> `bi_vertex_id` = `BI_PRELOAD_VERTEX_ID` = 60; the `raw_vertex_id + raw_vertex_offset` split (`pan_nir_lower_vertex_id.c`) is only for non-IDVS (Bifrost) flows |
| gl_InstanceID | **r61** of the same shader | `bifrost_compile.c:2049` `BI_PRELOAD_INSTANCE_ID` = 61; no base instance added |
| (r59, r62, r63) | r59 = INTERNAL_ID (the LEA_BUF_IMM source of every IDVS store, already lowered), r62 = draw id, r63 = view id | not needed |
| gl_FrontFacing | **r58**, the fragment "primitive flags" word: `(flags & 1) == 0` means front. On arch 10 (G610) the word is compared whole; arch >= 11 first masks bit 0 with `LSHIFT_AND` | `bifrost_compile.c:2017-2030`, `BI_PRELOAD_PRIMITIVE_FLAGS` = 58 |
| gl_PointCoord | `LD_VAR_SPECIAL.point` (a special varying, no slot of the VS) | as above, done |

What the frame supplies:

- gl_VertexID: nothing beyond what the draw already carries. Mesa's CSF draw
  sets `IDVS.VERTEX_OFFSET` to `draw->start` (non-indexed) or `index_bias`
  (indexed) (`pan_csf.c:1706-1717`), and INDEX_OFFSET/INSTANCE_OFFSET 0; the
  frame has these registers (`src/mali/frame.rs:257-259`, `INVALID_INDEXED_FIRST_VERTEX`
  at :132). No FAU word, no sysval: the shader reads r60. Case 57 (indexed,
  reversed indices) relies on r60 being the element value, not the position in
  the draw (that is what the case checks, gl_test.c:6530-6538); the index
  path already exists (frame.rs INDEX_BUFFER_SIZE).
  The program descriptors' **preload** mask must include r60 (and r59): the
  position shader of every draw preloads `0x38<<56` (r59,r60,r61); the varying
  shader of builtin_43 preloads `0x18<<56` (r59, r60; r61 not needed), of
  builtin_47 `0x38<<56`. This is a compiler output (`Program.preload`), and the
  frame writes whatever the compiler says.
- gl_InstanceID: the same r61, **plus an instanced draw**: the frame refuses
  `instance_count > 1` (`NOT_BUILT_INSTANCED`, `frame.rs:1951`; the draw's
  `instance_count` is carried at frame.rs:1329, `INSTANCE_COUNT` register :257
  = Mesa's `IDVS.INSTANCE_COUNT`, pan_csf.c:1706). The per-instance
  attributes (`offs`, `icol` advance per instance) need attribute records
  with an instance stride / divisor: the backend reports
  `max_instance_divisor: 0` (`src/mali/backend.rs:2728`, "divisor 1 only");
  case 141 needs the divisor past 1 (Mesa's attribute records use the
  instance-divisor `MALI_ATTRIBUTE_TYPE_1D_MODULUS` / POT-divisor forms;
  `pan_cmdstream.c:2533`) and a base-instance adjustment (:2533-2542).
  Not studied further: needs an instanced shim reference (see above).
- gl_FrontFacing: nothing per draw but the rasteriser's winding state: the
  frame already sets `front_face_ccw: d.front_cw != d.flip_y`
  (frame.rs:2937), which decides the flag the hardware writes into r58.
  The fragment program descriptor's preload mask must include r58
  (`0x34<<56` in builtin_49: r58, r60, r61, against `0x30<<56` of a
  shader without it).

## Mesa's words (exact; full lists in the JSON files)

**gl_VertexID, builtin_43 (`vid.vert`)** - varying shader, preload
`0x1800000000000000`, 32 registers; the position shader is the ordinary
`gl_Position = pos` one (builtin_43: 24 words, `LD_ATTR_IMM`, `FRCP`, ...,
preload `0x3800000000000000`). The `% 4` is an AND with 3 (a power of two),
and the compares are an `ICMP_OR` chain:

```
00b4c001108bc07c LSHIFT_AND.i32 r0, r60^, 0x0.b00, u5.w1      ; i = gl_VertexID & 3  (u5.w1 = constant 3, FAU word 11)
00f0c1009cc0c800 ICMP_OR.u32.eq.m1 r1, r0, 0x3020100.b3, 0x0  ; i == 3
00f0c2008001c000 ICMP_OR.u32.eq.m1 r2, r0, 0x0, r1            ; i == 0 || i == 3
0150c200d0c0c042 CSEL.u32.eq r2, r2^, 0x0, 0x0, 0x3F800000    ; ? 1.0 : 0.0
00f0c3009c01c400 ICMP_OR.u32.eq.m1 r3, r0, 0x1000000.b3, r1   ; i == 1 || i == 3
0150c300d0c0c043 CSEL.u32.eq r3, r3^, 0x0, 0x0, 0x3F800000
00f0c0009841c840 ICMP_OR.u32.eq.m1 r0, r0^, 0x3020100.b2, r1^ ; i == 2 || i == 3
0150c000d0c0c040 CSEL.u32.eq r0, r0^, 0x0, 0x0, 0x3F800000
085e840400000d7b LEA_BUF_IMM.slot0.wait0 @r4:r5, r59^, table:0xD, index:0x0
... three estream STOREs, of r2, r3 and r0 (v_col.w = 1.0 is a constant the
fragment shader supplies) ...
```

(the whole region is `builtin_43.expect.json`, `vs.varying`, 18 words).
Fragment, 8 words: `LD_VAR_BUF_IMM.f32.v3`, `V2F32_TO_V2F16`, `FADD`,
`ATEST`, `MKVEC.v2i16`, `BLEND.v4.f16`, the shape of any 3-lane varying colour. Note the varying program only exists because `v_col` is written; the
integer ops are the open part of the lowering (see "shared lowering").

**gl_VertexID with a loop-bounded fragment, builtin_98 (`uniloop.frag`)** -
vertex programs identical to builtin_43's position shader (no varying: the
shader writes none); the fragment is an `int` loop (`IADD_IMM`,
`S32_TO_F32`, `FCMP_OR.f32.ge.m1`, `BRANCHZ.reconverge`, `FROUND.f32.rtn`,
`F32_TO_S32.rtz`, `ICMP_OR.u32.eq.m1`, `CSEL.u32.eq`), 37 words, FAU
`{0,0,0,0, 16.0, 1/16, -16.0, 1/255*16}`; case 98's refusal after the
built-in is therefore the integers/loop (study-intbig, study-loop).

**gl_VertexID with a dynamic uniform index, builtin_45 (`dynidx.frag`)** -
fragment `LD_PKA.i128.unsigned` (r5 = `LSHIFT_OR(idx + i, 0x7060504.b00, 0)`,
the uniform array read through a `LD_PKA` of the push table), a loop of 4
`FADD.f32` per pass; 21 words. Blocked by the uniform array (study-uniarray,
case 108 shape) as well as the vertex id.

**gl_InstanceID, builtin_47 (`inst.vert`)** - position shader, preload
`0x3800000000000000`:

```
0066800412113d3c LD_ATTR_IMM.v2.f32.slot0 @r0:r1, r60, r61, index:0x1, table:0x1   ; offs (index 1) fetched with (vertex id r60, instance id r61)
1866820452017d7c LD_ATTR_IMM.v2.f32.slot1.wait01 @r2:r3, r60^, r61^, index:0x0, table:0x1 ; pos.xy
00b2c0000840d942 FMA.f32 r0, r2^, 0x34000000.h1, r0^          ; pos.x * 0.25 + offs.x  (0x34000000.h1 = 0.25 half-word constant)
00b2c1000841d943 FMA.f32 r1, r3^, 0x34000000.h1, r1^
00b2c00000818040 FMA.f32 r0, r0^, u0.w0, u0.w1                ; viewport scale/offset
00b2c10000838241 FMA.f32 r1, r1^, u1.w0, u1.w1
205e820480000d7b LEA_BUF_IMM.slot2.wait2 @r2:r3, r59^, table:0xD, index:0x0
... 5 stores (x, y, depth u2.w0, w = 1.0) ...
```

Varying shader (`vs.varying`, 24 words, preload `0x38<<56`): the instance
compare reads **r61**:

```
0066800832213d7c LD_ATTR_IMM.v4.f32.slot0 @r0:r1:r2:r3, r60^, r61, index:0x2, table:0x1   ; icol
08f0c4009cc0c87d ICMP_OR.u32.eq.m1.wait0 r4, r61^, 0x3020100.b3, 0x0   ; gl_InstanceID == 3
00b2c50408c0da00 FMA.f32 r5, r0, 0x38000000.h1, 0x0.neg                ; icol.x * 0.5
0150c0004540c004 CSEL.u32.eq r0, r4, 0x0, r0^, r5^                      ; ? icol*0.5 : icol
 ... same FMA/CSEL pair for y, z, w, interleaved with the estream stores
```

Fragment: `LD_VAR_BUF_IMM.f32.v4`, two `V2F32_TO_V2F16`, `ATEST`, `BLEND.v4.f16`
(7 words, `fs.fragment`).

**gl_FrontFacing, builtin_49 (`facing.frag`)** - fragment, preload
`0x3400000000000000` (r58, r60, r61), 9 words:

```
68f0c00080c0c07a ICMP_OR.u32.eq.m1.discard r0, r58^, 0x0, 0x0   ; r0 = (flags == 0) = front facing (arch 10: no mask)
00b9c1013800c0db MUX.v2i16 r1, 0x3C000000.h11, 0x0, r0.h00      ; (0, 1.0) pair: blue, alpha  -> green/red select
40b9c0012c40dbc0 MUX.v2i16.wait0126 r0, 0x0, 0x3C000000.h11, r0^.h00
087dbc0200ead07c ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
00a1c00000054041 MKVEC.v2i16 r0, r1^.h0, r0^.h0
4891c100000000db MOV.i32.wait r1, 0x3C000000
787f4004333c00f0 BLEND.slot0.v4.f16.end @r0:r1, blend_descriptor_0.w0, r60, target:0x0
0110f000000000c0 IADD_IMM.i32 r48, 0x0, #0x0
002fc1100000f1c0 BRANCHZI.eq.absolute 0x0, blend_descriptor_0.w1
```

The vertex programs of builtin_49 are builtin_43's position shader (the
`v_col` varying is not read by facing.frag, so Mesa removes it: no varying
program).

## Which cases share a lowering

1. **gl_VertexID, `vid.vert`**: cases 43, 57, 44, 45, 49, 98 all start with
   the same vertex shader. One new `LoadInput(VertexId)` arm in `nir.rs`
   (preload r60) opens the stage for six cases; it still needs the shader's
   integer arithmetic: `% 4`, `== k`, `||`, bool-to-float `?:`
   (`MaliError::IntValue`, nir.rs:1351, today refuses `ConstI32`) - the
   `LSHIFT_AND` / `ICMP_OR.m1` / `CSEL.u32.eq` words above. That integer form is the
   one study-intbig covers; it is the real work of this group.
2. **gl_InstanceID, `inst.vert`**: cases 47, 51, 141: the same arm for
   `InstanceId` (r61) + a single `ICMP_OR`/`CSEL` pair per lane, plus
   instanced draws in the frame and backend (above).
3. **gl_FrontFacing**: case 49 only; one fragment arm (`ICMP_OR ... r58, 0`
   with `MUX.v2i16`); independent of 1 except that its vertex shader is
   `vid.vert`.

## Implementation order (most sweep PASS first)

1. `VertexId` + the integer `% pot`, `==`, `||`, `?:` forms of `vid.vert`
   (nir.rs:1358; nir.rs:1351 for the ints; backend preload r60): unlocks the
   vertex stage of 6 cases. With the fragment `vcolor.frag` (already lowered)
   cases **43** and **57** should then PASS (57 needs nothing more of the
   frame: indexed u16 draws exist, no first vertex).
2. `FrontFacing` (nir_fs.rs:1393, preload r58): case **49** PASS (its second
   part, `hdrsample.frag` sampling a texture the first pass renders to, is
   the case-44 / texture shape already passing).
3. Case **44** needs, besides 1, an RGBA16F render target (a frame/format
   item, not a built-in); case **45** needs the uniform array (study-uniarray,
   case 108) and an int uniform (`n`, `idx`); case **98** needs the int loop
   (study-loop, study-intbig). Do these after their own items.
4. `InstanceId` (nir.rs:1358, preload r61) + instanced draws + attribute
   divisors in the frame (`frame.rs:1951`, `backend.rs:2728`): cases **47**,
   **51**, **141** (141 also the divisor past 1). Needs an instanced shim
   reference first (`gl_ref.py --instances`).

## The exact words to match

Per file, `tools/mali/pan_ref_diff.py diff <pan_shc stdout> <pan_shc stderr> --expect tools/mali/testdata/builtin_<n>.expect.json`
once pan_shc produces them. The contents that are specific to these built-ins:

- vertex programs: preload `0x3800000000000000` (position) and `0x18..` /
  `0x38..` (varying: r59,r60 / r59,r60,r61 - the mask follows which of
  r60, r61 the program reads, so a program that reads neither would differ);
  `work_reg_count` 32; vid.vert's constant 3 is FAU word 11 (u5.w1),
  promoted, not a sysval; the position shaders' FAU words 8-10 (4, 8, 12) are
  the store offsets as in every position program.
- fragment `facing.frag`: preload `0x3400000000000000`; the first word
  `68f0c00080c0c07a`, then the two `MUX.v2i16` words `00b9c1013800c0db` and
  `40b9c0012c40dbc0` (the `.h11` constants are 1.0 as f16 pairs `0x3C000000`).

## Caveats for the lowering items

- gl_VertexID of an indexed draw: Mesa does not add `index_bias` in the shader;
  the draw's `VERTEX_OFFSET` does. If the frame leaves VERTEX_OFFSET 0 for
  indexed draws (it refuses a bias, frame.rs:132) r60 is the raw element.
- r61 is read as an integer by `ICMP_OR` directly; no conversion.
- `gl_FrontFacing` on a framebuffer object: the board's FBO winding is
  inverted by the frame (`front_face_ccw` frame.rs:2884-2937); the case checks
  the colour on the default framebuffer and on the FBO of the `hdrsample`
  second pass.
- References are taken with a zero-filled attribute set, one instance, 6
  vertices: the `fau` words of the draw (viewport, offsets) are the shim's.

## Cases 44, 51, 57, 141 (study-builtin2)

The four cases the 2026-10-24 sweep
(`0dce654a8:tools/mali/board-ref/2026-10-24-refusals/ours/15-panthor-gltest--sweep-1.log`)
still refuses with the built-in-input message, after study-builtin recorded 43,
45, 47, 49 and 98. Each has its own reference,
`tools/mali/testdata/builtin_{44,51,57,141}.{vert,frag,expect.json}`, made from
the case's shaders in `driver/tools/pvrlab/gl_test.c`, with Mesa 26.2.1 on
drm-shim exactly as above (`$CONNECTED_DEPS_DIR/mesa-shim`, the tree
`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`):

```
tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim tools/mali/testdata/builtin_<n>.vert tools/mali/testdata/builtin_<n>.frag /tmp/builtin_<n>
python3 -I tools/mali/pan_ref_diff.py extract /tmp/builtin_<n>.trace --draw > tools/mali/testdata/builtin_<n>.expect.json
```

(`<n>` = 44, 51, 57, 141; the `trace` key of the JSON is the file name, so the
rerun with `/tmp/builtin_<n>` reproduces the files byte for byte.)

| file | shaders | case's body (`gl_test.c`) |
|---|---|---|
| builtin_44 | vid.vert + vcolor.frag | `case_es3hdr` :6769, colour pass |
| builtin_57 | vid.vert + vcolor.frag | `case_es3vid_indexed` :6549 |
| builtin_51 | inst.vert + vcolor.frag | `case_es3heavy` :7448 |
| builtin_141 | inst.vert + vcolor.frag | `case_es3instdiv` :7100 |

**The result: the programs are the recorded ones.** Compared with
`builtin_43` and `builtin_47`, builtin_44 and builtin_57 differ only in the
`trace` name from builtin_43 (the same vertex-ID varying shader: `LSHIFT_AND r0,
r60^, 0x0.b00, u5.w1`, the `ICMP_OR`/`CSEL` chain, preload
`0x1800000000000000`, position preload `0x3800000000000000`, fragment preload
`0x3000000000000000`), and builtin_51 and builtin_141 differ only in the
`trace` name from builtin_47 (`gl_InstanceID == 3` as `ICMP_OR.u32.eq.m1 r4,
r61^, 0x3020100.b3, 0x0` + `CSEL`, preload of the varying shader
`0x3800000000000000`, `LD_ATTR_IMM` at r60/r61 for `offs` and `icol`). Mesa's
program does not depend on the draw's count, divisor or index type, so these
words are the ones for every draw of the cases. What the shim cannot give:
`gl_ref.py` binds only `attribute`-declared inputs of an ES 2 program, draws
`glDrawArrays(…, 0, 6)` once and has no `--instances` option, so the
**attribute and buffer descriptors, the index and instance registers of the
instanced draws are not in these files**. They come from Mesa's source below
(and the board dump `tools/mali/board-dump/instsplit-2026-10-24.txt` for
divisor 1), not from a trace.

### Case 57, es3vid_indexed: gl_VertexID of `glDrawElements`

Reads `gl_VertexID` only (vid.vert, the position and varying shaders: r60). The
words are builtin_43's. Nothing differs from 43 in the programs; the draw is an
indexed `u16` one (indices 5,4,3,2,1,0, first 0, no bias), whose frame
descriptors already exist (`frame_indexed_tests.rs`). r60 for an indexed draw
is the element value (Mesa adds `index_bias` through `VERTEX_OFFSET`, 0 here),
which is what the case checks (top-left = vertex 5's colour). Lowering
`IoSlot::VertexId` as in builtin_43 unlocks 57 together with 43; nothing else
is needed.

### Case 44, es3hdr: gl_VertexID in the colour pass, RGBA16F target

Its colour pass draws vid.vert + vcolor.frag (the quad's two triangles) into an
RGBA16F texture; the second pass (cube.vert + hdrsample.frag) reads no built-in
and is the texture shape that already passes. The built-in is again
`gl_VertexID` through r60 and the words are builtin_43's. What differs from 43
is not in the shaders: the target is float16, so the render-target and blend
descriptors change (the `rgba16f` format of frame-f16rt, case 54), not the
programs. The shim's draw is on RGBA8, so builtin_44 is a reference for the
program words only. A lowering of `VertexId` unlocks the colour pass of 44; the case passes only
with the RGBA16F target item (frame-f16rt) as well.

### Case 51, es3heavy: gl_InstanceID, 1 to 2048 instances, divisor 1

inst.vert reads `gl_InstanceID` (r61; the `== 3` test) and three attributes
(`pos` per vertex, `offs`, `icol` per instance). The words are builtin_47's.
The draw is indexed (`u16`, a 50x50 grid, 14406 indices) and instanced with
`glDrawElementsInstanced` counts 1, 16, 64, 256, 512, 1024, 2048 (and a
second phase; the case times them). Frame needs: divisor 1 and instanced
draws are built (frame-instance, case 52: `Attribute` `1D POT Divisor`,
`Frequency: Instance`, `Divisor R/E/D` 0, the attribute's offset folded into
its buffer, `frame.rs:2699-2725`; `INSTANCE_COUNT` register `frame.rs:2993`).
What is new is only the size: the 2048-instance buffer (48 KiB: 2048 records
of 24 bytes) and 50x50 indexed vertices must fit the data arena (#1385 (b), done).
So the lowering of `InstanceId` (preload r61, as in builtin_47) is the only
missing piece, as for 47.

### Case 141, es3instdiv: gl_InstanceID with a divisor of 2

inst.vert again, the same words as builtin_47/51. The draw differs in two
ways the frame must handle: `glVertexAttribDivisor(1, 2)` and `(2, 2)` (the
`offs` and `icol` rows are one element per **two** instances; 4 instances of
2 elements), in both a `glDrawArraysInstanced(…, 0, 6, 4)` and a
`glDrawElementsInstanced` (u16 indices 0,1,2,0,2,3). The instance buffer is
two 24-byte records, so instance 3 reads record 1, which is what the case
checks (instances 0-1 red, instance 3 half-green at 0.5 alpha).

What Mesa does for a divisor of 2 on Valhall (arch >= 9),
`panfrost_pack_attribute`, `src/gallium/drivers/panfrost/pan_cmdstream.c:4083-4116`
(packed when the vertex elements are created, no draw state involved):

- divisor 0: `1D`, Frequency Vertex, Offset enable (what the frame builds for
  per-vertex attributes);
- a **power-of-two** divisor (including 1): `attribute_type =
  MALI_ATTRIBUTE_TYPE_1D_POT_DIVISOR`, `frequency = INSTANCE`, **`divisor_r =
  ctz(divisor)`**: for 2, `Divisor R` = 1 (genxml `Attribute.divisor_r`, bits
  56..=60, `src/mali/genxml/v10.rs:3493`); `Divisor E` and `D` are 0. The frame
  today writes `Divisor R` 0 (divisor 1);
- any other divisor: `1D_NPOT_DIVISOR` with `divisor_d =
  pan_compute_npot_divisor(divisor, &divisor_r, &divisor_e)` (`Divisor E` bit
  61, `Divisor D` bits 160..=191). Not needed by 141 (a divisor of 2) but the
  same function; the frame refuses it now.
- no padded-count product: `hw_divisor = padded_count * divisor` is the
  pre-v9 buffer-descriptor path (`pan_cmdstream.c:2372-2451`), not used on
  Valhall.

Also on the draw side: for an instanced draw with a divisor, `src_offset -=
src_stride * offset_start` if `instance_count > 1` (`:2541`, only the Bifrost
attribute emitter), and `base_instance * stride / divisor` is added for a
non-zero base instance (`:2533`); both are 0 here (no first instance, no
index-bias start). Mesa's buffer descriptor for the attribute is the ordinary
one (`address = buffer + offset`, `size`), which the frame already writes for
divisor 1.

So the frame (`src/mali/frame.rs`) must add, for 141:

1. in `check_draw`, accept `instance_divisor` 2 (a power of two) instead of
   refusing everything above 1 (`NOT_BUILT_INSTANCE_DIVISOR`, `frame.rs:1962`);
2. in the attribute descriptor (`frame.rs:2699-2714`), `divisor_r =
   instance_divisor.trailing_zeros()` for a power of two;
3. in the backend (`backend.rs:1969`, `max_instance_divisor` 0 at `:2844`),
   pass the core's divisor through (raise `max_instance_divisor`) instead of
   the clamp to 1. Whether the GL core then keeps handing divisor 2 depends on
   how `GlesAttrib.divisor` is stored; with `max_instance_divisor` 0 it
   emulates a divisor by expanding the buffer, and the Mali then never sees
   2 (that would also pass 141 without a frame change, at a copy's cost).

Verification that is not in these files: Mesa's exact attribute words for
divisor 2 (`Divisor R` = 1) are derived from the source above, not from a
trace, because `gl_ref.py` cannot draw an instanced ES 3 shader. An
`--instances N` / divisor option in `gl_ref.py` would make it a trace; until
then the board is the check.

### Which cases a lowering of the existing builtin_* forms unlocks

| lowering | cases it unlocks | what else the case needs |
|---|---|---|
| `VertexId` (r60) as in builtin_43 | **57** (nothing more), **43** | 44: the RGBA16F target (frame-f16rt) |
| `InstanceId` (r61) as in builtin_47 | **51** (nothing more: instanced draws, divisor 1 and the arena exist), **47** | 141: a divisor of 2 (items 1-3 above) |
| `FrontFacing` (r58) as in builtin_49 | 49 only | none of these four |
