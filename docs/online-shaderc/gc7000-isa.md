<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# The Vivante GC7000 instruction set, as this core uses it

GC7000UL — model `0x7000`, revision `0x6204`, product `0x70003`, HALTI5 —
on the Toradex Verdin i.MX8M Plus.

This is the reference the online shader compiler (`src/shaderc/online/`, task
#55) encodes to for the **first** target. It is the sibling of `isa.md`, which
documents the PowerVR Rogue USC and is now reference material only.

The machine-readable half is [`isa/gc7000-isa.json`](isa/gc7000-isa.json);
`gc7000-abi.md` covers what the driver loads before a shader runs.

Every statement below is either **measured** against the 309 shaders this
library ships, or marked **unverified**. Nothing is believed because a vendor
tool says so.

## Where the encodings come from

Mesa's `src/etnaviv/isa/etnaviv.xml` is an isaspec — an XML description of the
GC instruction set, MIT licensed, © 2023 Igalia S.L. It is the only complete
public one.

`tools/gc7000-isa/extract.py` reads that file **from wherever Mesa sits on the
build host** (`--mesa`, `$MESA_TREE`, default `~/pvr-lab/mesa-build/mesa-26.2.1`)
and writes the encoding facts out in our own schema. Mesa is not vendored:
nothing of theirs is copied into this repository, and the table records the
input's SHA-256 so a changed input is visible rather than silent.

    tools/gc7000-isa/extract.py -o docs/online-shaderc/isa/gc7000-isa.json --stats
    tools/gc7000-isa/extract.py -o docs/online-shaderc/isa/gc7000-isa.json --check

The schema deliberately mirrors the PowerVR side (`tools/isa-extract/`) so both
consumers are one generic table walker. The two instruction sets have almost
nothing in common — Rogue has variable-length instruction *groups* with
execution phases, the GC7000 has fixed-width instructions and none of that —
but the job the table does for an encoder and a decoder is the same.

**Is the description core-specific?** `etnaviv.xml` carries no per-model
gating. One description covers the GC family, and what differs per model lives
in the driver's feature bits, not in these bit layouts. That is a claim about
Mesa's source; the corpus check below is what makes it credible for *this* core.

## The shape of an instruction

**Fixed 128 bits — four dwords, little-endian, bit 0 being the least
significant bit of the first dword.** No groups, no phases, no padding, no
variable length. A shader is a flat array of 128-bit instructions, and a dword
count that is not a multiple of four is malformed.

The table holds:

| | count |
|---|---|
| enums | 8 |
| bitsets | 121 (31 abstract, 90 concrete) |
| instructions | 90 |

isaspec describes this as an **inheritance tree**. `#instruction` (size 128) is
the root and carries the fields every instruction has; abstract bitsets whose
names begin with `#` refine it (`#instruction-alu`, then
`#instruction-alu-src0-src1-src2`, and so on, by which operands are present);
the 90 leaves are the instructions. A leaf's own XML carries almost nothing —
usually just its opcode bits:

```xml
<bitset name="add" extends="#instruction-alu-src0-src2">
    <pattern low="0" high="5">000001</pattern>  <!-- OPC -->
    <pattern pos="80">0</pattern>               <!-- OPCODE_BIT6 -->
</bitset>
```

Everything else — where the sources are, where the destination is, the
condition, the type — comes from the chain above it. **The extractor flattens
that chain**, so each instruction in the table carries its complete field and
pattern set and a consumer never walks `extends` or gets the order wrong.

Three element kinds make up a bitset:

- **`field`** — a named bit range (`pos`, or `low`/`high`) with a type: `uint`,
  `int`, `bool`, `float`, or an enum such as `#cond`, `#reg_group`, `#swiz`.
- **`pattern`** — fixed bits, written MSB first, where **`x` means don't care**.
  The extractor turns each into a `value` and a `mask`, so a decoder matches
  with two ANDs and never has to know which bits were spelled out.
- **`derived`** — a value assembled from several fields. The extractor keeps the
  expression **verbatim** rather than paraphrasing it, because it is the
  authority on how the pieces combine. The one that matters most is the
  instruction type:

      TYPE = {TYPE_BIT2} << 2 | {TYPE_BIT01}

  with `TYPE_BIT2` at bit 53 and `TYPE_BIT01` at bits 95:94 — **a three-bit
  field split across the instruction**, which is the first thing a hand-written
  decoder gets wrong.

## The opcode is not a decode key

The opcode is seven bits: `OPC` at bits 5:0 plus `OPCODE_BIT6` at **bit 80**.
Already split, and already a trap. But the more important point is stronger:

**Three opcodes name more than one instruction.**

| opcode | instructions | separated by |
|---|---|---|
| 22 | `branch`, `branch_unary`, `branch_binary` | bit 43, then bit 70 |
| 23 | `texkill`, `texkill_unary`, `texkill_binary` | bit 43, then bit 70 |
| 127 | `bit_findlsb`, `bit_findmsb` | bits 108:100 — `0xb` vs `0xc` |

In each case the members differ in how many sources they take, and the
"extended" pair carries a sub-opcode in the high bits. A decoder that switches
on the seven-bit opcode alone therefore cannot work.

**Matching the full pinned pattern set does work, and this is checked rather
than asserted.** `extract.py` folds every instruction's patterns into one
128-bit (value, mask) pair and tests all 4,005 pairs for a word that would
satisfy both:

    pairs NOT separable by their full pattern set: 0   (decode is unambiguous)

So the rule for a decoder is: match on the whole pattern set, not the opcode.
The rule for an encoder is that it already knows its instruction and does not
care. And the check is not theoretical — `branch`, `branch_unary` and `texkill`
all occur in the corpus below and are separated correctly there.

## Verification: the whole corpus decodes

`tools/gc7000-isa/decode.py --blobs` is a table-driven reference decoder. It
exists to test the table; the shipping decoder and encoder are Rust, in
`src/shaderc/online/`.

It reads every shader in `src/shaderc/vivante_blobs.rs` and
`src/vivante/clear_shader.rs` — the offline table this library ships and draws
with today — and decodes each 128-bit instruction. Run of 2026-09-22:

```
shaders      309
instructions 13854
  decoded to exactly one instruction: 13854
  no match  (table cannot read):      0
  ambiguous (table cannot choose):    0
  distinct opcodes seen: 38 of 90 in the table
```

**13,854 of 13,854.** Nothing unreadable, nothing ambiguous, and every shader a
whole number of instructions. That last one is not free: it independently
confirms the 128-bit width, the four-dword grouping and the little-endian dword
order, because any of those being wrong would leave a remainder or produce
words that match nothing.

The opcode histogram is also a sanity check, and it passes one: `mad` (3358),
`mul` (2584), `mov` (2165), `add` (1091), `select`, `sqrt`, `cmp`, `rcp`,
`texld` (280) — the shape of compiled GLSL, not noise. A wrong bit numbering
would not produce a plausible instruction mix.

This is a far wider check than the PowerVR side got, where only four
hand-verified blobs were available. It is not, however, a check that the
*meaning* is right: it shows the table can read every instruction we ship and
name exactly one, not that the name is the one the hardware acts on. Silicon
decides that, at M3.

### Two independent readings of the XML agree

A second extractor (`tools/etna-isa/extract.py`) was written over the
same `etnaviv.xml`, independently, before this one landed. Rather than pick one
and move on, the two were diffed — which turns a duplication into the best
structural evidence available, because two people reading the same description
and disagreeing nowhere is worth more than either reading alone.

| compared | result |
|---|---|
| instruction names | identical, 90 of 90 |
| field sets (name, low, high) per instruction | identical, **0 of 90 differ** |
| folded decode constraint (value, mask over 128 bits) | identical, **0 of 90 differ** |
| top-level counts | both 121 bitsets, 90 instructions, 8 enums |

**And the diff found a real gap in this one.** Their extraction kept the
`<param>` children of a field; this one dropped them. They matter: a field whose
type is itself a bitset — `DST` is an `#instruction-dst`, `SRCn` an
`#instruction-src` — is decoded using values taken from elsewhere in the
instruction, and `DST` needs `DST_USE` and `COMPS`. Without them a consumer
cannot expand the nested operand bitsets at all. 246 field entries carry params;
this extractor now keeps all 246, matching theirs exactly. The corpus check was
re-run afterwards and is unchanged at 13,854 of 13,854.

### Mesa's disassembler reads it the same way

The strongest evidence available, and the one the PowerVR side got only from a
recorded disassembly: **a different tool, reading the same bytes, live.**

Mesa's own `etnaviv-disasm` is built from `src/etnaviv/isa/disasm.c` — the
isaspec decoder, not our table:

    cd <mesa>/build-etnaviv && ninja src/etnaviv/isa/etnaviv-disasm
    ./src/etnaviv/isa/etnaviv-disasm <raw 128-bit-aligned binary>

Run on `color_uv.frag`'s first three instructions and on `clear.vert`, against
our table's reading of the identical words:

| | Mesa `etnaviv-disasm` | our table |
|---|---|---|
| 1 | `mov t2.xy__, void, void, t2.xyxx` | `mov t2.xy__, void, void, t2.xyxx` |
| 2 | `mov t2.__z_, void, void, 1.0:f20` | `mov t2.__z_, void, void, immed0.xwww` |
| 3 | `mul t1, t1.xyzw, t2.xyzz, void` | `mul t1.xyzw, t1.xyzw, t2.xyzz, void` |
| 4 | `nop void, void, void, void` | `nop` |

Opcodes, destination registers, write masks, source registers, register groups
and **swizzles** all agree, including `t2.xyxx` and `t2.xyzz` — which exercises
the nested `#instruction-dst` and `#instruction-src` bitsets and the `params`
that connect them to their companion fields. `DST=3088` unpacks as `AMODE=0`,
`REG=2`, `COMPS=0b0011`, i.e. `t2.xy__`, which is what Mesa prints.

**The two differences are rendering conventions, not decode**, and both are
worth knowing before writing a listing:

- Mesa abbreviates a full write mask, printing `t1` where the mask is `xyzw`.
- Mesa recognises `RGROUP = immed` (7) and renders the **value**, `1.0:f20` — a
  20-bit float — instead of a register and a swizzle. Our reading correctly
  identifies the group as `immed`; the bits it then shows as `0.xwww` are the
  immediate, not a register. **A listing must special-case an immediate
  source**, or it will print a plausible register that does not exist.

#### Over the whole corpus: 13,854 of 13,854, zero disagreements

`tools/gc7000-isa/crosscheck.py` runs Mesa's disassembler over **every shader we
ship** and compares it to our reading, instruction by instruction:

```
shaders compared      309
instructions compared 13854
  base opcode AGREES  13854
  base opcode DISAGREES 0
  Mesa printed no line for a slot we decoded: 0
  Mesa printed a slot we did not:             0
```

The last two zeros matter as much as the first: the two tools agree on **where
the instruction boundaries are**, not only on what sits at them.

**The run found a second real gap in the extractor, which is why it was worth
running over the corpus and not a sample.** The first pass reported 208
disagreements, every one of them `branch_unary` (ours) against `branch`
(Mesa's). Neither reading was wrong about the bits — the isaspec carries an
explicit `displayname` attribute, and this extractor was dropping it:

| bitset | `displayname` |
|---|---|
| `branch_unary`, `branch_binary` | `branch` |
| `texkill_unary`, `texkill_binary` | `texkill` |

So the **bitset name is not the mnemonic**. Those families are one instruction
with different operand counts, and isaspec says so. Note what does *not* carry a
`displayname`: `bit_findlsb` and `bit_findmsb` are genuinely different
mnemonics that happen to share a 7-bit opcode, separated by the sub-opcode at
bits 108:100. A listing that prints bitset names disagrees with Mesa on exactly
the 208 `branch_unary` occurrences in this corpus; one that prints
`instructions[*].mnemonic` agrees everywhere.

**The one remaining difference is modifiers, and it is ours to close, not a
decode gap.** Mesa renders the display template, so 2,967 of the 13,854 carry a
suffix our listing does not print yet — 32 distinct forms, headed by
`imullo0.s32` (533), `add.s32` (347), `select.nz.u32` (303), `mov.sat` (195).
The fields behind them are in the table (`#cond`, the split `TYPE`, `SAT`); only
the rendering is missing. That is the listing work, and the cross-check will
grow a second column when it lands.

Also confirmed: the all-zero word decodes to `nop` in Mesa's reading too. That
was worth checking rather than assuming — an all-zero word decoding to a legal
instruction is exactly where a table that is too permissive looks like a table
that is right.

### Instruction coverage, for M1

38 of the 90 instructions occur in the corpus; the other 52 are transcribed and
unexercised:

`atomic_add`, `atomic_and`, `atomic_cmp_xchg`, `atomic_max`, `atomic_min`,
`atomic_or`, `atomic_xchg`, `atomic_xor`, `barrier`, `bit_extract`,
`bit_findlsb`, `bit_findmsb`, `bit_insert`, `bit_rev`, `branch_any`,
`branch_binary`, `call`, `clamp0_max`, `conv`, `div`, `dp4`, `f2irnd`, `i2i`,
`iaddsat`, `idiv0`, `imadlo0`, `imadlosat0`, `img_load`, `img_store`, `imod`,
`imulhi0`, `leadzero`, `load`, `lshift`, `movai`, `norm_dp2`, `norm_dp3`,
`norm_dp4`, `norm_mul`, `popcount`, `ret`, `rotate`, `set`, `store`, `swizzle`,
`texkill_binary`, `texkill_unary`, `texldb`, `texldd`, `texldgpcf`,
`texldlpcf`, `tg4`.

Most are ES 3.x or compute (`atomic_*`, `img_*`, `barrier`, `bit_*`), which an
ES 2.0 compiler never emits. Some are ordinary and their absence is a fact
about this corpus rather than about the hardware — `dp4` is missing while `dp3`
and `dp2` are common, and `div` is missing while `rcp` is not, which says the
offline compiler lowers those. An online compiler is free to emit them, and
M1's round trip should cover them even though nothing here exercises them.

### Decoding and rendering are different properties

The corpus check above says the table can READ every instruction the driver
ships. It says nothing about whether the table can SHOW one, and the two have
now diverged FOUR times — twice in each of the two extractions, which is the
part of the count that matters: neither of us was the careless one.

Every one of these was invisible to decode, which passed 13,854 of 13,854
throughout, and every one was found only by a second tool that had to *print*
what the table held:

| gap | whose extractor | found by | what a reader lost |
|---|---|---|---|
| `<param>` children of a field (246 entries) | pvrgpu-platform | diffing two independent extractions | which fields accompany an operand — the decoder never asks |
| `displayname`, so the mnemonic fell back to the bitset name | pvrgpu-platform | the corpus cross-check against `etnaviv-disasm`, which produced 208 false disagreements | every instruction whose printed name differs from its bitset's |
| `display` and `override` | the listing | building a listing that had to render an instruction at all | the rendering template, and the per-variant overrides of it |
| the six `<template>` fragments and the `derived` expressions | the listing | a listing that had to render a modifier suffix and compose `TYPE` | the suffixes, in order, and any field the table computes rather than stores |

The pattern is the same each time: a decoder consumes a table by asking "which
bits mean what", and never notices that a name, a companion field or a
composition rule is missing, because it does not need them to choose a variant
and read the operands out. A printer needs all of them.

So **a decoder agreeing with the corpus is not evidence that the table is
complete.** It is evidence that the table is complete *for decoding*. The
second tool is not a nicety; it is the only thing that has ever found these,
and all four were real omissions in an extractor rather than gaps in the source
XML. An extractor complete for one property can be silently incomplete for the
other, and only a consumer asking the other question finds out.

The current standard of agreement, from `tools/gc7000-isa/crosscheck.py` over
every shader the driver ships: 11,204 of 11,424 instructions identical to
Mesa's own disassembler, 4 disagreements (float FORMATTING — `9.18354962e-41`
against the decimal expansion of the same bits, nothing about what an
instruction is or reads), and 216 branch lines where Mesa prints a label and we
print a number, counted apart rather than folded into either column because
label assignment needs a whole-shader pass that a per-instruction printer
cannot honestly invent. That separation was itself learned the hard way: folded
in, the labels produced a false "length mismatch" that silently dropped 8,058
instructions from the comparison. 2,430 further instructions are outside that denominator
and say so: Mesa's disassembler caps at 1,000 lines and truncates two Qt3D
shaders whose true lengths are 1,145 and 1,283.

## Register classes and operands

From the enums in the table:

- **`#reg_group`** selects the file an operand names — temporaries, uniforms and
  the internal groups. `SRCn_RGROUP` carries it per source.
- **`#reg_addressing_mode`** (`SRCn_AMODE`, `DST_AMODE`) selects direct or
  indexed access.
- **`#swiz`** is a two-bit component select, `x`/`y`/`z`/`w`, four of them per
  source swizzle (`#src-swizzle`, size 8).
- **`#cond`** is the condition code: `.true`, `.gt`, `.lt`, `.ge`, `.le`, `.eq`,
  `.ne`, `.and`, `.or`, `.xor`, `.not`, `.nz`, `.gez`, `.gz`, `.lez`, `.lz`,
  and `.selmsb` at 22.
- **`#thread`** and the `LOW_HALF`/`HIGH_HALF` bits carry the thread mode.
- **`#type`** is the operand type, through the split `TYPE` derived field above.

Each instruction has one destination (`#instruction-dst`, 14 bits: register,
write mask `COMPS`, addressing mode, and a `DST_USE` bit) and up to three
sources (`#instruction-src`, 10 bits each: register, register group,
addressing mode, and a `USE` bit, with the swizzle carried separately).

`SAT` (bit 11) saturates the result; `DST_FULL`, `SKPHP`, `PMODE`, `RMODE`,
`DENORM`, `LOCAL` and `LEFT_SHIFT` are the modifier bits the display templates
print. **`PMODE` is stored inverted** — Mesa's encoder maps it as `!src->pmode`
— which is the kind of thing that produces a shader that is right except in one
mode.

## The source slot is not the operand ordinal

The three source slots are named `SRC0`, `SRC1` and `SRC2`, and an operation
has *some subset* of them. The subset is not "the first N":

| operation | slots it has |
|---|---|
| `mov`, `floor`, `ceil`, `sign`, the transcendentals | `SRC2` only |
| `and`, `or`, `add` | `SRC0` and `SRC2` |
| `mul` | `SRC0` and `SRC1` |
| `cmp`, `select`, `mad` | all three |

So a `mov`'s one operand goes in `SRC2`, and a generator that fills slots by
ordinal writes `SRC0` instead -- which **encodes perfectly and reads a register
nobody wrote**. It does not fail; it computes something else. The display
template is what settles each case, because it prints the operands in slot
order and writes `void` where the operation has no slot
(`{INSTR_ALU} {DST}, {SRC0}, void, {SRC2}`).

This is handled in one place, `OpId::slot_of_operand`, and nothing else should
know it. It is written down here because the failure is silent and the first
three tests written against `builtins.rs` all made exactly this mistake.

## Which conditions are actually used

`#cond` has sixteen values plus `.selmsb`, but a compiler for this machine
needs four. Two independent sources say so.

Mesa's NIR table (`etnaviv_compiler_nir_emit.c`) maps comparisons to `cmp`
with `EQ`, `NE`, `GE` and `LT` and contains **no `LE` or `GT` row at all**, for
either the float or the integer forms. Our corpus agrees: of the 438 `cmp`
instructions in the 13,854 shipped, the conditions are `.lt` 192, `.ge` 122,
`.eq` 77, `.ne` 47, and **zero** of the other twelve.

So `a > b` and `a <= b` are reached by swapping the operands, not by a
different condition. A compiler that emits `.le` would be the first program on
this core to try it, and nothing here says what would happen.

`select` is the other heavy user of the field, with a different three: `.gt`
and `.lt` for the min/max family and `.nz` for the conditional select. The
operand shape distinguishes them -- of 606 selects, all 303 `.gt`/`.lt` have
`src2 == src0` and all 303 `.nz` do not, a partition with no exceptions.

## Immediates are inside the instruction

An operand whose `SRCn_RGROUP` is `immed` (7) is not a register: the payload
occupies what would otherwise be the register number, the swizzle, the two
modifier bits and the low bit of `SRCn_AMODE`, 20 bits in all, with
`IMMED_TYPE` (`SRC_AMODE >> 1`) saying whether they are a float, a signed or an
unsigned value. A float immediate carries the **top 20 bits of the f32**, so
1.0, 0.5 and the small powers of two are exact and 0.1 cannot be represented at
all -- `Src::imm_f32` refuses it rather than rounding, because a silently
rounded constant is a colour nobody asked for.

Not rare: 2,130 of the 28,055 source operands in the corpus are immediates,
most of them on `cmp` (634), `add` (290), `select` (268) and `mad` (220).

## What is NOT established

- **Nothing here has been checked on silicon.** The corpus check shows the
  table reads what we ship; it does not show the hardware agrees with the
  naming. M3 is the first time that is tested.
- **52 of 90 instructions are unexercised** (listed above).
- **The encode direction is untested.** Decoding 13,854 instructions says the
  table can read them; it does not say a re-encode reproduces the bytes. That
  is M1's round trip and it is the check that closes this gap.
- ~~No cross-check against Mesa's own disassembler.~~ **Done** — see "Mesa's
  disassembler reads it the same way" below.
- `etnaviv.xml`'s `encode` blocks reference `struct etna_inst *`, Mesa's own IR.
  The table keeps those expressions verbatim as documentation of intent; they
  are **not** a specification our encoder can execute, and nothing in
  `src/shaderc/online/` should try to.

## Regenerating

    tools/gc7000-isa/extract.py -o docs/online-shaderc/isa/gc7000-isa.json --stats
    tools/gc7000-isa/decode.py --blobs        # must stay at 13854/13854

The second command is the gate: if a table change stops the shaders this
library ships from decoding, the change is wrong.
