<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): a float uniform array read at a run-time index, Mesa's design

Issue #1385 (b), plan #153 items `study-uniarray` (this note) and `uniarray`
(the lowering of gl_test case 108 `uniidx`, programs A, B and C of
`tools/shaders/es100/uniidx_{a,b,c}.frag`, refused with `MaliError::UniformArray`,
"Valhall: a uniform array is not lowered yet"). Companion of
`docs/notes/mali-uniform-index.md`, which covers the vec4 array and the
`LD_PKA` design; this note adds what a **float** array changes. Written before
the lowering; the mediump form landed with `uniarray`, the fp32 forms with
`uniarray-int` (section "The fp32 forms").

Reference: `tools/mali/testdata/case108_uniidx.{vert,frag,expect.json}`, program
A (dEQP `float_dynamic_read`: four reads of `mediump float u_arr[4]` at four
`mediump int` uniforms), Mesa 26.2.1 on drm-shim, no `--uniform-values`
(`gl_ref.py` refuses it for an int uniform, so every FAU word is 0 and the
words are told apart by the disassembly). The Mesa dump below is
`BIFROST_MESA_DEBUG=shaders,shaderdb` (the script keeps it in `<out>.stderr`).

## What Mesa does

1. **The block.** The default uniform block is UBO 0, `vec4[2] uniform_0`
   (32 bytes): the four ints are packed in the first vec4 (bytes 0..15, one
   dword each), the array follows at byte 16 (`decl_var ubo vec4[2]
   uniform_0`). `num_ubos: 2`.
2. **The element type is fp16, the stride is a dword.** A `mediump float[4]`
   is `float16_t[4] u_arr`, yet each element keeps its **4-byte** slot (packed
   uniform storage counts dwords): NIR is
   `ishl idx, 2; iadd 0x10, that; load_ubo (0, offset)` with a **16-bit**
   `load_ubo` (`align_mul=4`, `range_base=16, range=16`). A highp float
   array (not run; **pin on the shim**) would be the same with a 32-bit load.
3. **The indices are pushed, the array is not.** One 4-component `load_ubo
   (0, 0)` reads the four ints; all four words go to the FAU (`u2.w0`,
   `u2.w1`, `u3.w0`, `u3.w1`, "4 uniforms used"). The element reads are not
   constant-offset, so they stay `LD_PKA` from the UBO (the same split as
   `mali-uniform-index.md` section 2): the frame must **upload** UBO 0 (`ubo_mask`
   bit 0) although the FAU holds the pushed words. The pushed words of the
   trace's FAU table are only the ints; no array word is pushed.
4. **The NIR** (fragment, after lowering; `ishl`/`iadd`/`load_ubo` per read,
   verbatim from the first two reads and the sum):

```
32x4   %2 = @load_ubo (%1 (0x0), %1 (0x0)) (access=none, align_mul=1073741824, align_offset=0, range_base=0, range=16)
32     %3 = ishl %2.x, %0 (0x2)
32     %5 = iadd %4 (0x10), %3
32     %6 = ishl %2.y, %0 (0x2)
32     %7 = iadd %4 (0x10), %6
16     %8 = @load_ubo (%1 (0x0), %5) (access=none, align_mul=4, align_offset=0, range_base=16, range=16)
16     %9 = @load_ubo (%1 (0x0), %7) (access=none, align_mul=4, align_offset=0, range_base=16, range=16)
16    %10 = fadd %8, %9
```

   (`%2.z` and `%2.w` follow the same way for the third and fourth read; the
   variable is `decl_var uniform mediump float16_t[4] u_arr`, `decl_var ubo
   vec4[2] uniform_0`.)
5. **The code** (fragment, `fs.fragment` of the expect file, 22 instructions):

```
MOV.i32.discard r0, 0x3020100               ; the byte-index table
LSHIFT_OR.i32 r1, u2.w0, r0.b22, 0x0        ; idx << 2 (the table's byte 2 is 2)
IADD_IMM.i32 r1, r1^, #0x10                 ; + the array's byte offset
(the same pair for u2.w1)
LD_PKA.i16.unsigned.slot0 @r1, r1^, 0x0     ; one 16-bit load per read, UBO 0
LD_PKA.i16.unsigned.slot1.wait01 @r2, r2^, 0x0
FADD.v2f16 r1, r1^.h00, r2^.h00             ; the sum, in half precision
... (u3.w0, u3.w1 the same, the FADDs chained)
ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
U16_TO_U32 r0, r0^.h0                       ; the half red, widened for BLEND
MOV.i32.wait r1, 0x3C000000                 ; green 0 | blue 1.0 half pair
BLEND.slot1.v4.f16.end @r0:r1, blend_descriptor_0.w0, r60, target:0x0
```

   `work_reg_count` 32, `preload` `0x3000000000000000`, FAU 8 words, all 0.
   The mode operand of every `LD_PKA` is the constant 0 (resource table 0,
   index 0). The reads are scheduled `slot0/1/2` with `wait` flags as the
   existing `uniidx_d` reference (`LD_PKA.i64`).

## What differs from the vec4 case already lowered (`uniidx_d`)

| | vec4 array (`uniidx_d`, lowered) | mediump float array (case108) |
|---|---|---|
| element in the block | a mediump vec4: 2 dwords (8 bytes), stride 8 | one **dword**, stride 4 |
| offset | `idx << 3` (`LSHIFT_OR u2.w0, r0.b33`) | `idx << 2` (`LSHIFT_OR u2.w0, r0.b22`; NIR `ishl 2`) **plus** `IADD_IMM #0x10` |
| array base | byte 0 (the array starts the block) | byte 16 (after the int vec4): an `IADD_IMM` |
| load | `LD_PKA.i64.unsigned` (the 8-byte element: four halves) | `LD_PKA.i16.unsigned`, one half |
| result use | two words split | a half in `.h00`, `FADD.v2f16` |
| index source | one pushed int | up to four pushed ints (`u2.w0 ... u3.w1`) |

(The `.b33` and `.b22` selectors are as the expect files print them; the
`idx << 3` of the vec4 case is `mali-uniform-index.md`'s, the `<< 2` here is
NIR's `ishl`.)

## What yakogl would need (not implemented)

- **Admit the array.** `nir.rs:552` (`array_len > 1` in the vertex `uniform_of`)
  and `nir_fs.rs:1630-1660` `uniform_extent` refuse everything but an
  array of vec4. A float array is `u.gl_type == GL_FLOAT`, `array_len` elements,
  `dwords == array_len` (the front end's dword count; **check on the board
  front end**: the IR here may count the packed fp16 pair differently).
  `uniform_extent` must return its extent in Mesa's block.
- **The block layout.** `select.rs` `UboLayout` (line 226) packs a uniform per
  Mesa's `PackedDriverUniformStorage`: a float array puts element `i` at dword
  `base + i` (stride 1 dword, one value per dword, **not** packed in pairs).
  The four ints before it fill the first vec4; the array's base is the next
  dword-aligned slot, here dword 4 (byte 16). The uploaded block
  (`UboRow`, `src/shaderc/online/mali/mod.rs:584` `ubo`) must carry the array's
  elements at that stride, and the block size is `ceil(dwords/4)` vec4s (2
  here).
- **The read.** `nir_fs.rs` `ubo_load` (line 2962) is the only
  run-time-index lowering: it requires `lv.ubo.word(dword) == Some(0)` (the
  array starts the block), shifts by `trailing_zeros(4 * words)` and emits
  `LdPka { words }`, which is 32 or 64 bits. It must learn (a) a base byte
  other than 0, an `Iadd` of the array's byte offset after the shift (Mesa's
  `IADD_IMM #0x10`; `emit.rs:83` selects only `Ishl` for this path, an `Iadd`
  with an immediate is already a row), (b) the 16-bit load
  (`LD_PKA.i16.unsigned`: `Op::LdPka` carries `words: u8` today; a half
  needs a bit width or a new variant, and `emit.rs` its encoding), (c) a
  shift of 2 for a one-dword element, (d) the result as a half in `.h00`
  for the following `FADD.v2f16` (the `lv.ubo_words` pair holds two words; a
  half read fills one lane).
- **Index sources.** Each read uses its own pushed int; `ubo_load` handles one
  `Kind::Uniform` index (`Val::Lane(i, 0)`). The four ints must all reach the
  FAU in `fau.rs` `push_ubo` (line 112) in Mesa's order (`u2.w0, u2.w1,
  u3.w0, u3.w1`: the first four dwords of the block).
- **The upload.** `src/mali/frame.rs` (`ubo_mask` bit 0, line 485;
  `emit_stage_uniforms` line 2367) already uploads UBO 0 when the program has
  `ubo` rows (the vec4 case). A float array needs the array's dwords in the
  upload: `src/mali/backend.rs` `uniform_words` (line 1859) reads the draw's
  GL storage by dword; the program's `ubo` rows must map each block dword to a
  front-end dword. glUniform1fv of an array already lands as consecutive
  dwords in the GL storage (case 108 uploads with `glUniform1fv(loc, 4, arr)`).
- **Reference and test data.** `tools/mali/testdata/cases.txt` takes a row for
  `case108_uniidx` (`vs.position=equal fs.fragment=...`) once pan_shc or
  yakogl matches; `tools/mali/corpus-verdicts.txt` moves `uniidx_a/b/c.frag`
  from the "uniform array" refusal to `ok` when the lowering lands (that file
  is the `uniarray` item's, not this one's).

## The fp32 forms (plan #153 `uniarray-int`, lowered)

The shim settled the highp and vertex-stage arrays
(`tools/mali/testdata/uniarray_highp.*`, `uniarray_vs.*`); both are lowered
byte-equal (tests.rs
`a_highp_float_array_read_at_int_uniforms_is_byte_equal_to_mesa_26_2_1`,
`a_vertex_float_array_read_at_an_int_uniform_is_byte_equal_to_mesa_26_2_1`).

- **The index arithmetic is the mediump one.** `LSHIFT_OR.i32 r, <int>,
  0x3020100.b22, 0x0` then `IADD_IMM.i32 r, r^, #<4 * base word>` (no
  `IADD_IMM` when the array is word 0); only the load is wider:
  `LD_PKA.i32.unsigned` of UBO 0 (`lane` 0, `extend` none, as the `.i16`).
- **The block is fp32**, one word an element: uniarray_highp's ints at words
  0-3 and `u_arr` at words 4-7 (`#0x10`), uniarray_vs's `ui_two` at word 0
  and `u_arr` at words 1-4 (`#0x4`). The int index words are pushed to the
  FAU (the fragment's `u2.w0 .. u3.w1` after the blend constants, the
  vertex's `u5.w1` after the viewport words and the store offsets); the
  array is not pushed, so the record carries the block (`ubo` rows) and the
  frame uploads it for that stage.
- **Highp fragment:** the reads are placed as the mediump ones (each offset
  where its read stands, each load moved down to its first reader:
  offsets 0 and 1, both loads, `FADD.f32`, then offset, load, `FADD.f32` per
  read), the sum converted at the store (`FADD.f32 r0.h0, r0^, 0x0.neg`).
- **Vertex stage:** the read stands in the varying program; the varying
  lane it feeds has no attribute behind it, yet Mesa's link neither
  propagates nor moves it (as a texture look-up's): it is promoted to flat,
  `FMA.f32 r0, r0^, 0x0, r0^` before its store. The constant lanes of
  `vec4(x, 0.0, 0.0, 1.0)` go to the fragment shader.
- **An int uniform is admitted only as an index.** Any other read of it
  (`float(i)`, integer arithmetic) keeps the uniform-type refusal
  (`nir.rs` `index_only`).

## Open

- uniarray_vs's fragment shader, linked: Mesa reads the promoted lane flat
  (`LD_VAR_BUF_IMM.f32 ... .src_flat32 ..., 0x0`, a flat descriptor format
  on the frame side), and its colour pair of that lane and 0.0 is
  `U16_TO_U32` (refused today, `MaliError::ColourZeroHalf`). Neither is the
  array's; the vertex shader is checked built for the linked read.
- A vec4 array at an int uniform in an fp32 stage (corpus `uniidx_e.vert`,
  `idxtex.{vert,frag}`) and a float array read at a constant index, or
  whose only reader is the colour's store, in a highp shader: no reference,
  refused (`MaliError::UniformIndexed`, `MaliError::UniformArray`).
- The `.b22` selector vs `uniidx_d`'s `.b33` in the table above: take both from
  the expect files, not from this note.
- Reproducing (verified on the unmodified script): with the shim built by
  `drm_shim_ref.sh build` in `<work>`,
  `tools/mali/drm_shim_ref.sh run <work> tools/mali/testdata/case108_uniidx.vert tools/mali/testdata/case108_uniidx.frag <dir>/case108_uniidx`
  then `python3 -I tools/mali/pan_ref_diff.py extract <dir>/case108_uniidx.trace --draw > tools/mali/testdata/case108_uniidx.expect.json`
  gives the committed file byte for byte (`vs.position` and `fs.fragment`
  words included). The first attempt on another host failed
  with `eglInitialize failed`; the cause there was that host's shim install,
  not the script, which is unchanged. No step was edited by hand.
  `drm_shim_ref.sh run` has since dropped `stdbuf` for `python3 -u`; the
  same two commands still give the committed file byte for byte.
