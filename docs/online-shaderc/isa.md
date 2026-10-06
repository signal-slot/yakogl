<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# The Rogue USC instruction set, as this core uses it

BVNC 36.53.104.796 — PowerVR BXS-4-64, Toradex Verdin AM62P.

This is the reference the online shader compiler (`src/shaderc/online/`, task
#55) encodes to. It is a working document: every statement below is either
**measured** against a program this driver already runs on silicon, or marked
**unverified** and left as an open question. Nothing here is believed because a
vendor tool says so.

The machine-readable half is [`isa/usc-isa.json`](isa/usc-isa.json).
`abi.md` covers the other half — what the driver loads into which register
before a program runs.

## Where the encodings come from, and what that means

Mesa's `src/imagination/pco/pco_isa.py` describes the USC encodings in a small
Python DSL (`pco_pygen_common.py`). It is Imagination's own description of
their hardware, MIT licensed, and it is the only complete public one.

`tools/isa-extract/pco_isa_extract.py` imports that file **from wherever Mesa
sits on the build host** (`--mesa`, `$MESA_TREE`, default
`~/pvr-lab/mesa-build/mesa-26.2.1`) and writes out the encoding facts in our own
schema. Mesa is not vendored: nothing of theirs is copied into this repository,
and the generated JSON records the input files' SHA-256 so a changed input is
visible rather than silent.

    tools/isa-extract/pco_isa_extract.py -o docs/online-shaderc/isa/usc-isa.json --stats
    tools/isa-extract/pco_isa_extract.py -o docs/online-shaderc/isa/usc-isa.json --check

What that buys us is a transcription that cannot go wrong by hand over ~3,000
lines of bit slices. What it does **not** buy us is any assurance the
description matches this core. That is what the verification section is for.

### Is the description core-specific?

**No, and this was checked rather than assumed.** `pco_isa.py` contains no
BVNC test and no feature test of any kind. Every `PVR_HAS_FEATURE` use in the
whole of pco is in the NIR lowering passes, not the encodings:

| file | line | feature |
|---|---|---|
| `pco_nir_tex.c` | 471, 939 | `tpu_extended_integer_lookup` |
| `pco_trans_nir.c` | 221 | `slc_mcu_cache_controls` |
| `pco_trans_nir.c` | 528 | `usc_itrsmp_enhanced` |

So the bit layouts are common across the cores pco supports, and what differs
per core is which instructions the compiler chooses to emit. That is a claim
about pco's source, and it is consistent with — but not proven by — the
verification below.

## What the table contains

Eight bit sets, 102 encoding variants, 73 enums, 89 field types.

| bit set | pieces | fields | variants | what it encodes |
|---|---|---|---|---|
| `igrp_hdr` | 17 | 17 | 4 | instruction-group header |
| `src` | 36 | 36 | 16 | source operands |
| `iss` | 5 | 5 | 1 | internal source selects |
| `dst` | 20 | 19 | 5 | destination operands |
| `main` | 69 | 70 | 23 | main ALU instructions |
| `backend` | 76 | 78 | 35 | backend (UVSW, DMA, sampling, emit) |
| `bitwise` | 19 | 18 | 5 | bitwise ALU |
| `ctrl` | 56 | 51 | 13 | control flow |

### The four object kinds

**Bit set** — a byte-oriented layout. Its *pieces* are named bit slices
`(byte, hi:lo)`, each within one byte. Its *fields* are logical values, each
built from one or more pieces **listed most significant first**, so
concatenating them in the order given reconstructs the value. A field may carry
a `reserved` value, meaning it is that value in every variant.

**Bit struct (variant)** — one encoding of a bit set: a subset of the fields,
where some are *pinned* to constants. The pinned fields are the discriminators.
Total width is byte-aligned by construction and pieces never overlap, so each
variant's byte length is exact.

**Field type** — base type (`bool`, `uint`, `enum`), the width **as encoded**,
and, when the stored form differs from the logical one, `dec_bits` plus the
transform. Four transforms appear, and a decoder must undo all four:

| transform | encoded | logical |
|---|---|---|
| `{} - 1` | 0..3 | 1..4 |
| `{} % 16` | 0..15 | 1..16, with 0 meaning **16** |
| `{} >> 1` | 31 bits | a 32-bit even offset |
| `{} >> 2` | 6 bits | an 8-bit multiple of 4 |

The `% 16` one is the trap: a stored 0 in a `length` field means 16, not 0.

**Enum** — named values. A *subtype* is a parent enum narrowed to the values
that still fit in fewer bits; it carries no elements of its own.

## Instruction groups

A USC program is a sequence of **instruction groups**, not of instructions. A
group issues one instruction per pipeline phase, and the phases execute in
order within the group.

Layout, from `pco_binary.c:pco_encode_igrp()`:

    [ header, 2 or 3 bytes ]
    [ instructions, phases in REVERSE order: backend, p2*, p1, p0 ]
    [ lower srcs ] [ upper srcs ] [ iss ] [ dests ]
    [ word padding ] [ alignment padding ]

Word padding is a single `0xff`. Alignment padding is a lead byte `0xf0 | n`
where `n` is the run length in 2-byte words, followed by `0xff` bytes.

Two header fields carry the group's shape, and both are load-bearing:

- **`length`** counts **2-byte words of the whole group**, not bytes
  (`pco_group_instrs.c:117`, `igrp->hdr.length = igrp->enc.len.total / 2`).
  Through the `% 16` transform, a stored 0 means 16 words = 32 bytes. Reading
  this as a byte count is the first mistake a decoder makes.
- **`da`**, the decode assist, is the byte offset at which the source/dest area
  begins — but it is *not* simply the end of the instructions
  (`pco_group_instrs.c:26`, `calc_da()`):
  - `main`: header bytes plus the instruction bytes of the phases **after p1**
    only. A group whose only instruction is in p0 or p1 therefore has
    `da == header length`, with instruction bytes sitting past `da`.
  - `bitwise`: header bytes plus **every** phase's instruction bytes.
  - `control`: **0** when the group has no sources and no destinations at all;
    otherwise header bytes plus the ctrl instruction.

### Header variants

`ext` (byte 1, bit 7) selects a 2-byte or 3-byte header. When extended,
`alutype` (byte 2, bits 6:5) picks the bit set the instructions belong to:
0 `main`, 2 `bitwise`, 3 `control`. Value 1 is not assigned.

| variant | bytes | pinned | settable |
|---|---|---|---|
| `main_brief` | 2 | `ext=0` | `da length oporg olchk w1p w0p cc` |
| `main` | 3 | `ext=1 alutype=main` | the above plus `end atom rpt` |
| `bitwise` | 3 | `ext=1 alutype=bitwise` | `opcnt` in place of `oporg`, plus `end atom rpt` |
| `control` | 3 | `ext=1 alutype=control opcnt=0` | `miscctl ctrlop`, no `end`/`rpt` |

`oporg` (3 bits) names which phases the group occupies: `p0`, `p2`, `be`,
`p0_p1`, `p0_p2`, `p0_p1_p2`, `p0_p2_be`, `p0_p1_p2_be`. `opcnt` is a bitmask
instead (p0=1, p1=2, p2=4). `rpt` is the repeat count, 1..4 through the
`{} - 1` transform: **one group can execute up to four times**, which is how a
vector move becomes a single group.

`end` marks the last group of the program. `cc` is the condition code,
assembled from the `ccext` and `cc` pieces.

## Verification against this driver's own programs

`tools/isa-extract/usc_decode.py --blobs` is a table-driven reference decoder.
It exists to test the table, not to ship: the shipping decoder and encoder are
Rust, in `src/shaderc/online/`.

It decodes the four hand-verified USC programs in `driver/src/`, which this
driver has run on silicon since bring-up. **All four decode completely, with
every byte accounted for.** Run of 2026-09-22:

```
== driver/src/pvr_usc_vs_passthrough_blob.h — passthrough VS
   32 bytes
  group @0x000  16 bytes  hdr=main(3B) alutype=main
    hdr: da=5 oporg=be rpt=3 end=0 cc=e1_zx atom=0 olchk=0 w0p=0 w1p=0
    [backend ] uvsw_write_imm           2B @+3   {'dsel': 'w0', 'imm_addr': 0}
    srcs/iss/dests: +5..+11 (6 bytes, 5 bytes 0xff pad)  da=5 as calc_da predicts
  group @0x010  16 bytes  hdr=main(3B) alutype=main
    hdr: da=5 oporg=be rpt=1 end=1 cc=e1_zx atom=0 olchk=0 w0p=0 w1p=0
    [backend ] uvsw_write_emit_endtask_imm 2B @+3  {'dsel': 'w0', 'imm_addr': 3}
    srcs/iss/dests: +5..+11 (6 bytes, 5 bytes 0xff pad)  da=5 as calc_da predicts
   groups=2 bytes accounted=32/32
```

with `pvr_usc_clear_fs_blob.h` one group (`p0` `sngl`/`byp`, `end=1`, 16/16
bytes), `pvr_usc_clear_fs2_blob.h` two such groups with `end` on the second
(32/32), and `pvr_usc_eot_blob.h` one group (`backend` `emitpix`, `freep=1`,
`end=1`, 16/16).

Three independent things agree, which is why this is worth more than a
successful parse:

1. **The program means what the file says it means.** The passthrough VS is
   documented as `gl_Position = vec4(in.xyz, 1.0)`. It decodes as a UVSW write
   of component 0 repeated **3** times, then a write of component **3** fused
   with emit-and-endtask — three components copied, then w, then the vertex is
   emitted and the task ends. Nothing about the decoder was arranged to produce
   that reading. Likewise the clear FS is a `byp` (bypass, i.e. a move) per
   output dword, one group per dword, and EOT is a single `emitpix`.
2. **`length` and the group boundaries agree.** Each group's declared length
   lands the next header exactly on a valid header, and the final group in each
   program — and only the final group — carries `end=1`. A wrong length model
   would put the second header mid-instruction.
3. **`da` agrees with an independently computed value.** `da` is read out of the
   header; the expected value is re-derived from the instruction lengths the
   variant matching produced. The two paths share no input. They match on all
   six groups, including the two `p0`-only cases where `da` is *less* than the
   instruction end — the case that would look like a bug if `calc_da`'s
   phase rule were not implemented.

On (3): the first run of this check reported a mismatch on the clear FS. The
mismatch was the instrument — a naive `da == instruction end` — and not the
table. The fix was to implement `calc_da` faithfully. Recorded because the
tempting move was to treat the table as wrong.

### What this does NOT establish

- Only **5 of 102 variants** are exercised: `main` header, `sngl`,
  `uvsw_write_imm`, `uvsw_write_emit_endtask_imm`, `emitpix`. The other 97 are
  transcribed and unverified. M1's round-trip over the 2,042 table blobs
  is what will exercise the rest, and the coverage list it
  produces belongs next to this section.
- The **operand areas decode, but are not yet split automatically.** See
  "Operands" below: the first source and the destination of both test programs
  decode correctly, and they match a disassembly produced by Mesa's own
  disassembler. Splitting the area into lower srcs / upper srcs / iss / dests
  in general needs each op's source and destination counts, which live in
  `pco_map.py`. Until then the decoder reports the first source and leaves the
  rest as bytes.
- A cross-check against Mesa's disassembler has been done for **one** program,
  the clear FS, against the disassembly recorded in `src/pvr/blobs.rs` (now
  `tests/reference/pvr_internal.rs`) when the
  blob was captured (see "Operands" below). Running `pco_shc` over a table blob
  and diffing a full disassembly is still outstanding, and remains the widest
  form of this check.

## Operands

The bytes after the instructions hold, in this order: lower sources, upper
sources, the internal-source-select byte, destinations.

**The variants are self-describing.** This was the open question in the first
draft of this document, and it is now answered: `src` and `dst` variants are
picked out by a chain of pinned extension bits, exactly like the group header.

| bit set | discriminator chain |
|---|---|
| `src` | `ext0`, then `sel`, then `ext1`, `ext2`, `ext4` — 16 variants, 1 to 6 bytes |
| `dst` | `ext0` for the one-destination forms; `ext1`, `ext2` for the two-destination forms — 5 variants, 1 to 4 bytes |
| `iss` | none: a single fixed 1-byte struct, present or absent |

Two things are **not** in the bits, and a decoder must get them from context:

1. **Lower versus upper.** `1lo_1b6i` and `1up_1b6i` carry identical pinned
   bits, as do the other lower/upper pairs. They are distinguished by position:
   the lower sources are encoded first. This is not ambiguity, it is ordering.
2. **How many sources and destinations.** The variant tells you how many bytes
   *that* operand group occupies, but not how many groups there are. That comes
   from the ops in the group. Mesa never needs to ask, because
   `pco_igrp_src_variant()` and `pco_igrp_dest_variant()`
   (`pco_map.h.py:160`, `:284-287`) *choose* the variant from the bit widths the
   register bank and index actually need — the smallest form that fits. So the
   encoder picks by value and the decoder must split by op.

That is the remaining piece of mechanical work for a general decoder, and it is
`pco_map.py`'s per-op source and destination counts. It does not block M1 (the
encoder knows its op) or M3.

### Verified against the recorded disassembly

The clear-attach block of `src/pvr/blobs.rs` (now `tests/reference/pvr_internal.rs`,
"PCO final IR") records the clear FS's final IR, printed by Mesa's
own disassembler at the time the blob was captured:

    0000: { s0 = sh0, is4 = ft0 p0: mbyp ft0, s0; w0 = po0 }.olchk.end

Decoding those same 16 bytes with our table, by a path that never saw that
line, gives:

| our decode | the recorded disassembly |
|---|---|
| header `olchk=1`, `end=1` | `.olchk.end` |
| phase p0, `main` variant `sngl`, `sngl_op=byp` | `p0: mbyp` |
| src `1lo_3b11i_2m`: `sb0=shared`, `s0=0` | `s0 = sh0` |
| dst: bank `special`, index 32 = `pixout0` | `w0 = po0` |

Four independent agreements, across the header, the instruction, the source and
the destination. The source result is corroborated a second time by
the same block's prose, which states that this shader "reads the
clear colour from SHARED REGISTER 0 (sh0) and writes it to pixel-output
register 0 (po0)" — a statement written for the PDS author, long before this
table existed.

The passthrough VS's source decodes the same way and is equally load-bearing:
`sb0=vtxin`, `s0=0` — it reads **vertex input register 0**, which is what
`gl_Position = vec4(in.xyz, 1.0)` requires and what the DDMAD in
`abi.md` delivers there.

The two-dword clear FS adds a fifth agreement, and an independent one: its two
groups decode as `shared s0=0` and `shared s0=1`, and its header file states
"The clear colour is consumed from shared registers **sh0, sh1**"
(`driver/src/pvr_usc_clear_fs2_blob.h`). One group per output dword, reading
consecutive shared registers.

This is the strongest verification in this document. It is not a parse that
succeeded; it is our reading of the bytes agreeing, field by field, with a
reading produced by a different tool.

**One decode is not explained.** The passthrough VS's *second* group reports its
first source as bank `special`, index **64**, and 64 is not in the `SR` table
(which runs ...62, 63, then 96). That group is the one writing `w`, so a
constant source is what one would expect, but this document does not assert
what special register 64 is. It is recorded as unexplained rather than guessed
at, and it is a good first question for the `pco_shc` disassembly cross-check.

## Source modifiers, and the one that is not on every source

The USC applies modifiers **per source**, the way the GC7000 applies `SRC_NEG`
and `SRC_ABS` — so a negate or an absolute costs no instruction when its
consumer can carry it. The USC has a third one the GC7000 does not: **floor**.

Counted in the extracted table (`isa/usc-isa.json`), field names across all
variants:

| modifier | src0 | src1 | src2 |
|---|---:|---:|---:|
| negate (`sNneg`) | 41 | 10 | 21 |
| absolute (`sNabs`) | 41 | 28 | 17 |
| floor (`sNflr`) | 11 | **0** | 5 |

**`s1flr` does not exist.** Not "is rare" — the string appears zero times in
the whole table, while `s0flr` and `s2flr` appear eleven and five times. So the
floor modifier is available on sources 0 and 2 and not on source 1, and a
lowering that folds `Ffloor` into an arbitrary source will, one time in three,
produce an instruction the encoder cannot build.

This is the sort of asymmetry the reference does not state. Mesa's `pco` folds
`FNEG`, `FABS` and `FFLR` into a consumer's source
(`pco_opt.c`, `PCO_OP_FNEG`/`FABS`/`FFLR` → `pco_instr_src_has_neg`/`_abs`/
`_flr`), and it asks per source *index* without saying which indices can
answer yes — the question is settled by the table, not by the pass.

READ from pco rather than measured, and worth keeping separate from the counts
above:

- The fold is **opportunistic, not guaranteed**. `pco_opt.c` folds only into
  `FADD` and `FMUL` consumers, and only when the modifier is not wanted on both
  sources (swapping would achieve nothing). A negate whose consumer is an
  `FMAD` stays a real instruction.
- pco does not fold **two** modifiers onto one source: its own comment says
  "TODO: support cases where > 1 modifier can be applied (e.g. .abs.neg)". So
  `-|x|` is not one modified source there, whatever the encoding allows.
- `FNEG`, `FABS` and `FFLR` are **pseudo-ops** in pco's IR (`pco_ops.py`,
  `pseudo_op`) with no consumer in any `.c` file other than that folding pass,
  which is what makes the fold the only path they have.

What this means for `powervr/builtins.rs`: `Fneg` and `Fabs` are modifiers
first and instructions second, as on the GC7000; `Ffloor` is a modifier **on
two of the three sources** and an instruction otherwise; and none of the three
may be folded onto a source that already carries a modifier until somebody
establishes that the hardware allows the pair, because pco declines to.

## Open questions

**Decode is ambiguous for three variant pairs in `main`.** Pinned fields alone
do not pick a variant:

| variants | shared pinning |
|---|---|
| `fmad` / `movc` | `main_op=fmad_movc, ext0=0` |
| `fmad_ext` / `movc_ext` | `main_op=fmad_movc, ext0=1` |
| `int32_64` / `tst`, and their `_ext` forms | `main_op=int32_64_tst, ext0=…` |
| `pck` / `upck` | `main_op=sngl, ext0=1, sngl_op=pck` |

These pairs overlay the *same bits* with different meanings. pco never has to
resolve this because it encodes from an IR that already knows the op. A decoder
does. The phase slot is the likely discriminator — pco's `OP_PHASE` enum gives
`p2_pck`, `p2`, `p2_tst` and `p2_mov` distinct slot numbers (2, 2, 3, 4) — but
`oporg` has only one `p2` encoding, so how the hardware selects among the p2
sub-phases is **not yet established**. This is stated as an open question
rather than guessed at; `usc_decode.py` reports such a group as `AMBIGUOUS`
instead of picking silently. It does not affect M1's round-trip (the encoder
knows its op) and it does not affect M3 (neither a passthrough VS nor a solid
FS reaches these), so it is not on the critical path — but it must be closed
before the decoder is trusted as an oracle.

**Longest-match variant selection is a heuristic.** Where several variants match
and differ in length, the decoder takes the longest, on the reasoning that the
extension bits selecting a longer form are themselves pinned. That held for all
six groups above but has not been shown to hold in general.

## Register classes

Two bank encodings, both 3 bits, and they are **not the same mapping** — a
decoder that uses one for the other will silently address the wrong file:

| value | `regbank` (sources/dests) | `idxbank` (indexed access) |
|---|---|---|
| 0 | `special` | `temp` |
| 1 | `temp` | `vtxin` |
| 2 | `vtxin` | `coeff` |
| 3 | `coeff` | `shared` |
| 4 | `shared` | — |
| 5 | `coeff_alt` | `idx` |
| 6 | `idx0` | `coeff_alt` |
| 7 | `idx1` | `pixout` |

`regbank2` and `regbank1` are the 2-bit and 1-bit subtypes, holding only the
values that still fit — so a 1-bit bank field can name `special` or `temp` and
nothing else.

The banks, in the terms the rest of this project uses:

- **temp** — per-instance scratch. A program declares its count; the driver
  passes it to the PDS (`abi.md`).
- **vtxin** — vertex inputs, written by the vertex DMA the vertex PDS programs.
- **coeff** / **coeff_alt** — coefficient registers: the interpolation
  coefficients a fragment program reads varyings through, loaded by DOUTI.
  See `driver/docs/gpu-recipes/multivarying_coeff_recipe.md`.
- **shared** — shared/uniform registers, `sh0` upward. Uniforms and the 20-dword
  sampler descriptors live here; `driver/docs/gpu-recipes/shader_toolchain.md`
  is the existing statement of that layout and `abi.md` the fuller one.
- **special** — the special registers below.
- **idx0** / **idx1** — index registers for indexed register access.
- **pixout** — fragment outputs, reachable as special registers 32..35 and
  164..167, or as an `idxbank`.

### Special registers

The ones this driver's programs and its ABI touch (`SR` in `pco_isa.py`):

| number | name | note |
|---|---|---|
| 32..35 | `pixout0..3` | fragment colour outputs |
| 164..167 | `pixout4..7` | outputs 4..7 |
| 36..43 | `intl0..7` | internal registers; also named `slot7..slot0` — **the alias runs backwards**, `intl0` is `slot7` |
| 44 | `face_orient` / `back_face` | front/back facing |
| 45 | `cluster_num` | |
| 46 | `output_part` | |
| 47 | `task_id` | |
| 48 | `slot_num` | |
| 49, 50 | `tile_x_pix`, `tile_y_pix` | pixel coordinates within the tile |
| 51 | `inst_num` | instance number within the wave |
| 52 | `dm_task_type` | which data master issued the task |
| 53 | `samp_num` | sample number, for multisampling |
| 54..57, 119..122 | `tiled_ld_comp0..7` | tiled load components |
| 58..61, 123..126 | `tiled_st_comp0..7` | tiled store components |
| 62 | `batch_num` | |
| 63 | `inst_valid` | |
| 96 | `tile_xy` | |
| 97, 98 | `x_p`, `x_s` | |
| 100, 101 | `y_p`, `y_s` | |
| 102 | `wg_id` | compute work-group id |
| 103 | `sh_alloc_size` | shared-register allocation size |
| 104..111 | `global0..7` | |
| 112 | `local_addr_inst_num` | |
| 114..117 | `tile_x_p`, `tile_x_s`, `tile_y_p`, `tile_y_s` | |
| 118 | `render_tgt_id` | |
| 127 | `gpu_offset` | |
| 160 | `timer_80ns` | |

The `intl`/`slot` reversal is the kind of detail that produces a shader that
works for one component and not the next; it is written down for that reason.

## Execution model

Stated to the extent this document can support it; the parts that are inference
are marked.

- A program runs over **instances** in a wave. `inst_num` gives the instance
  and `inst_valid` says whether it is live; `samp_num` gives the sample.
- A group's **`rpt`** repeats the whole group 1..4 times. The passthrough VS
  uses `rpt=3` to write three position components from one group — measured
  above, and the clearest single piece of evidence for what `rpt` does.
- **Predication** is the header's `cc`, assembled from `ccext` and `cc`. Every
  group in all four of our programs carries `cc=e1_zx`, which is therefore the
  unconditional/default encoding on this core; the other values are
  transcribed but unexercised.
- `end` on the header ends the program; `uvsw_endtask` and the fused
  `uvsw_write_emit_endtask` end the task from the backend phase.

### UVSW: how a vertex program emits

The Unified Vertex Store Writer is the vertex program's output path, and the
passthrough VS is made of nothing else. Variants (`backend_op=uvsw`):

| variant | bytes | what it does |
|---|---|---|
| `uvsw_write_reg` / `uvsw_write_imm` | 2 | write one output, address from a register or an immediate |
| `uvsw_emit` / `uvsw_emit_imm` | 1 / 2 | emit the vertex |
| `uvsw_cut` / `uvsw_cut_imm` | 1 / 2 | cut the primitive |
| `uvsw_emit_cut` / `uvsw_emit_cut_imm` | 1 / 2 | both |
| `uvsw_endtask` | 1 | end the task |
| `uvsw_emit_endtask` | 1 | emit, then end |
| `uvsw_write_emit_endtask_reg` / `_imm` | 2 | write, emit, end — one instruction |

`dsel` selects the write port (`w0` in both of our groups) and `imm_addr` the
output slot: 0 for the first three components under `rpt=3`, 3 for the last.

### EMIT and the DOUTx interface

`emitpix` (`backend_op=emit`) is the fragment-side emit, and it is the whole of
`pvr_usc_eot_blob.h`: one group, `freep=1`, `end=1`.

The **DOUT\* operations are not USC instructions.** DOUTI, DOUTD, DOUTU and
DOUTT are issued by the *PDS* — a separate programmable sequencer that runs
before the USC program and loads its registers. They are generated by
`driver/src/pvr_pds_render_gen.c`. Where the USC meets them is the register
state they leave behind: coefficient registers for varyings, shared registers
for uniforms and sampler descriptors, vertex-input registers for attributes.
That boundary is `abi.md`'s subject, and it is the part `src/shaderc/online/`
has to honour exactly, because the driver programs the PDS side today and will
keep programming it.

The USC's own DMA-class backend instructions (`backend_op=dma`: `idf`, `ld`,
`st`, `smp*`, `atomic`) are a different mechanism — memory access issued by the
shader itself, including the `smp` sampling family whose five variants
(`smp_brief` through `smp_extabc`, 2 to 5 bytes) carry the texture-sampling
encoding. Unverified here.

## Regenerating

    tools/isa-extract/pco_isa_extract.py -o docs/online-shaderc/isa/usc-isa.json --stats
    tools/isa-extract/usc_decode.py --blobs        # must stay at 4/4, all bytes accounted

The second command is the gate: if a table change stops our own silicon-proven
programs decoding, the change is wrong.

## The register files: what is a limit and what is only a measurement

`powervr::alloc::Limits` takes four file sizes. Only one of them has a source
that states it. The other three are recorded here as open rather than filled
with a plausible number, because `Limits` is the bound an allocator refuses
against, and a bound invented to make a struct complete is worse than a missing
one: it refuses programs that would have run, or admits programs that will not.

### `temps` = 252, and this one is hard

The USC's temporary count reaches the hardware through the PDS `DOUTU`
instruction's `src0`, which this driver already builds for every kick
(`driver/src/pvr_pds_render_gen.c`, `pds_build_doutu_src0`):

```c
#define PDS_DOUTU_SRC0_TEMPS_SHIFT      35u
#define PDS_DOUTU_SRC0_TEMPS_MASK       0x3fu   /* 6 bits */
#define PDS_DOUTU_SRC0_TEMPS_ALIGNSIZE  4u

blocks = DIV_ROUND_UP(usc_temps, PDS_DOUTU_SRC0_TEMPS_ALIGNSIZE);
```

The field holds **blocks of four**, not temporaries, so the largest temporary
count the hardware can be told is `0x3f * 4` = **252**. This is not a
transcription from a reference driver: it is the encoding this driver uses on
silicon today, for every draw that has ever rendered on this part.

Two consequences worth stating where the number is.

The count is rounded **up** to a multiple of four, so a shader using 5
temporaries is declared 8. An allocator that packs to 5 gains nothing over one
that packs to 8, and the useful granularity of a temps bound is four.

And the write **masks**: `src0 |= (blocks & PDS_DOUTU_SRC0_TEMPS_MASK) << ...`,
guarded by an `assert` that a release build drops. A shader needing more than
252 temporaries is therefore encoded with a wrapped block count today rather
than refused -- the same defect class as the GC7000's `NUM_TEMPS` seam, and it
should become a refusal on the same argument.

### `shared`, `vtx_in`, `coeff`: no source states them

These are properties of the part (BXS-4-64, BVNC 36.53.104.796) and nothing in
this repository declares them. What exists is adjacent and does **not** answer
it: `PDS_DOUTD_SRC1_BSIZE` (13 bits) and `_AO` (15 bits) bound one DMA transfer
into the common store, not the size of the file it lands in, and
`shader_toolchain.md`'s worked examples give occupancy for particular shaders
(`coeffs=12`, `20`, `28`; `shareds=1`, `4`) which are demonstrated lower bounds
and no more.

The corpus can raise those lower bounds -- 2,042 blobs, 576,296 groups, every
source register named in them -- but reading a register number out of a source
section means decoding the variant-dependent bank and index fields, which is
the legalise/encode work rather than a measurement that can precede it. So
these three arrive with the encoder.

Until then `Limits` stays a caller parameter, and
a caller that does not know passes a bound it can defend.

## The pinned registers, per stage

From `driver/docs/gpu-recipes/shader_toolchain.md`, the ABI reference for
shaders this driver has actually run:

**Fragment.** The `1/w` iterator is allocated **first**, at `cf0..3`, before any
user varying; user varyings start at `cf4` and take one four-dword coefficient
set per scalar component. The colour output is `po0` (`po0..3` for `f32x4`).
When the shader runs in full mode it also reads a driver-supplied ISP-feedback
**meta control word at `sh0`**, which displaces the first uniform.

**Vertex.** Attributes arrive in `vi0..` in order, one register per attribute
dword, placed by the vertex-fetch DMA; outputs leave through `uvsw.write` to
slots rather than to a register the allocator can choose.

One thing deliberately **not** asserted here: gles2-core's reading that a
fragment shader's temporary `r0` holds `gl_FragCoord` (the diagnosis behind
fifteen black dEQP cases). That is about the temporary file and this reference
describes the coefficient file, so the two do not contradict -- but nothing in
this repository states it, and if it is true it is a pin an allocator must
respect. It needs their evidence written down before it becomes a rule here.
