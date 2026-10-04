<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The in-process shader compiler: design as it is built

YakoGL compiles GLSL ES 1.00 to PowerVR Rogue USC in the library itself, with
no host and no offline table. This document is written as the work happens,
so it records what exists and what is still a plan, and says which is which.

Feature `shaderc-online`, module `src/shaderc/online/`, `no_std` and
allocation-free like the rest of the crate.

## Where the milestones stand

The first GPU is the **Vivante GC7000UL** (i.MX8MP); the PowerVR Rogue work
below came first and stays as the reference and as a second gate.

| | | |
|---|---|---|
| M1 | the instruction set: decoder, encoder, disassembler | **done** for both GPUs, see below |
| M2 | GLSL ES 1.00 front end and reflection | es100-frontend |
| M3 | back-end module, ABI assembly, a triangle on the AM62P | gles2-core |
| M4 | `gl_test` regression set, then dEQP, through this path | |
| M5 | a Zephyr image with no offline table; then Vivante | |

## M1, GC7000 (the target)

```
vivante/isa_tables.rs   generated (tools/etna-isa/); one row per operation
vivante/isa.rs          what those tables mean, and the bit-level read and write
vivante/decode.rs       128-bit instructions -> operations and fields
vivante/encode.rs       back again, built from zero
vivante/listing.rs      one instruction as text (yakogl-tools etna-dis)
```

This machine is far simpler than the PowerVR. An instruction is a fixed 128
bits, four little-endian dwords. There is no instruction group, no phase
model, no padding and no framing to work out: each instruction pins its own
opcode, and the only question is which operation it is. The description
(Mesa's etnaviv isaspec XML) is a tree of bitsets — a root with the fields
every instruction has, intermediate bitsets per instruction shape, a leaf per
operation — and `tools/etna-isa/extract.py` flattens each leaf's inheritance
so an operation is just a mask, the values it pins, and a list of fields.

Measured by `cargo test --no-default-features
--features host-test,gpu-vivante,shaderc-online vivante::`:

* `round_trip_every_blob_in_the_offline_table` — **307 of 307 blobs, 13,852
  instructions**, decoded and written back byte for byte. The encoder builds
  each instruction from zero, so a bit the tables do not account for comes
  back zero and the comparison fails.
* `no_instruction_in_the_corpus_decodes_two_ways` — nothing is claimed by two
  operations.
* `every_operation_covers_all_128_bits` — checked on the tables, not on the
  corpus, so an operation nothing uses yet cannot hide a hole until the code
  generator emits it. All 90.
* `no_dont_care_bit_is_set_in_the_corpus` — see below.
* `isa_coverage` — 38 of the 90 operations appear, counted:
  `docs/online-shaderc/gc7000-isa-coverage.md`. `mad` 3,358, `mul` 2,584,
  `mov` 2,164, `add` 1,091, `select` 606 is the head of the list, and that is
  where a code generator starts.

### The builder: the direction a code generator needs

`vivante/build.rs` makes an instruction that never existed — name an
operation, give it a destination and sources, get 128 bits. Its operand model
is the machine's: a destination is a register index, a write mask and an
addressing mode; a source is a register file, an index, an addressing mode, a
per-component swizzle and the negate and absolute modifiers; the data type is
three bits the encoding splits across bit 53 and bits 95:94, which `ty()`
writes so no caller has to know. Nothing allocates, nothing panics: an
operation that does not exist, a field an operation does not have, or a value
too wide for its field is a `BuildError`.

Three tests, weakest to strongest:

* the bring-up triangle's shader (`driver/src/vivante/gc_shaders.h`) is a
  single NOP — the hardware loads attributes straight into the shader's
  temporaries and reads the outputs back out, so a pass-through needs no
  arithmetic — and the builder produces those exact words. Weak on its own,
  four zero dwords, and here because it is the one whose *meaning* is
  independently known: the driver has drawn a triangle with them on silicon.
* a field the operation does not have is an error rather than a silently
  dropped operand, and a value too wide is refused rather than truncated.
* **every one of the 13,854 instructions the driver ships is rebuilt from its
  operands, bit for bit**: decode it, read the operands back, build a fresh
  instruction from those operands alone, compare. A decode/encode round trip
  only shows the fields are preserved. This shows they can be *stated* — that
  a caller who knows nothing but "this operation, this destination, these
  sources, this type" reaches the same instruction, which is exactly what a
  code generator knows and nothing more.

### Don't-care bits, and why they have their own test

Seven operations declare bits the description marks `x`: bit 34 of the five
branch forms and of `call`, and nineteen bits of `store` and `img_store`.
They belong to no field, so nothing names their value. An encoder that builds
an instruction from zero would decide they were zero and say nothing about
it — code with no representation for "this was not modelled", substituting a
value indistinguishable from a real one. So the mask travels with the
operation, a decoded instruction carries those bits verbatim, and a test
records what they actually hold in the table today: nothing, across the 303
instructions that could have had one set. The first shader that sets one is
then a named failure rather than a silent change in what the compiler emits.

`every_operation_covers_all_128_bits` was written before this was known and is
what found it: it failed on seven operations, and the fix was to model the
bits rather than to widen the test.

## M1, PowerVR Rogue USC (the reference)

```
isa_tables.rs   generated (tools/pco-isa/); bit sets, forms, enums, operations
isa.rs          what those tables mean, and the bit-level read and write
decode.rs       USC bytes -> instruction groups
encode.rs       instruction groups -> USC bytes
listing.rs      one group as text (yakogl-tools usc-dis)
```

A shader is a sequence of **instruction groups**. A group's header gives its
length in 16-bit words, the ALU it is for (main, bitwise, control), and which
pipeline phases carry an instruction. The instructions follow in reverse
phase order, then the lower and upper source sections, the ISS byte and the
destination section, then padding.

Most of that is self-describing: a source section's leading extension bits
say how many sources it holds and therefore how long it is, an instruction
form is picked out by its operation bits, and the group's length is in the
header. Two things are not, and they are the whole design problem:

* **Which phases are present.** The header's `oporg` names a coarse group
  (`p0_p1_p2`), not the individual phase-2 slots. The decoder tries the
  possible phase sets and keeps the one that fits. This is not bookkeeping:
  the same form means a different operation in a different phase — main op 6
  is FMAD in phase 0 and MOVC in the move phase — so the phase has to be
  settled before the bytes can be read at all.
* **Which sections are present.** An ISS byte carries no discriminator
  whatsoever, so nothing in the bytes distinguishes "an ISS byte" from "a
  byte of padding". The rule is structural, and it comes from Mesa's variant
  selectors rather than from the bit layout: an ISS byte on every MAIN group
  and no other; a lower and an upper source section on every MAIN and BITWISE
  group, each at least one byte even when nothing is in it; on a CONTROL
  group, a section only where that half has a source; and a destination
  section exactly when the header says a write port is used.

Two rules keep the parse honest:

* **The decoder never guesses.** A group that reads two ways returns
  `DecodeError::Ambiguous` instead of picking one, and a test asserts that no
  group in the corpus is ambiguous. Without that, "it round-trips" would mean
  only "some reading of it round-trips".
* **An instruction form that names no operation is not a parse.** The form
  `sngl` is FRCP, FRSQ, FLOG, FEXP, MBYP and the derivatives at particular
  `sngl_op` values, and nothing at the rest; a form whose operation-selecting
  fields match no operation in that phase is something the compiler cannot
  have emitted. This is the rule that makes the parse unique: with it,
  2,042 of 2,042 blobs; without it, 1,834, with 208 blobs holding a group
  that reads two ways.

### What M1 is measured by

`cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online` over
the whole offline blob table:

* `round_trip_every_blob_in_the_offline_table` — 2,042 of 2,042 blobs,
  576,296 instruction groups, decoded into instructions and written back byte
  for byte. This is a test of the tables, not of the blobs: the only way a
  group re-encodes exactly is that every bit of it was accounted for by some
  field of the form the decoder chose, so a missing field, a misplaced bit
  piece or a wrong form all surface as a byte that differs. The encoder
  zeroes each region before writing it, so a bit no field covers cannot
  survive by accident.
* `no_group_in_the_corpus_decodes_two_ways` — the uniqueness above.
* `isa_coverage` — what the corpus uses, counted:
  `docs/online-shaderc/isa-coverage.md`. 48 operations and 31 instruction
  forms across those 576,296 groups. That file is the code generator's target
  list, and it is regenerated from this decoder so the two cannot drift.

The group count is quoted alongside the blob count on purpose: a decoder that
stopped after each blob's first group would report every blob as
round-tripped. 576,296 is also what an independent walk of the group framing
alone (header length field only, no instruction decoding) counts, which is a
second instrument agreeing with the first.

## Two readings of the encoding, compared

`tools/isa-extract/pco_isa_extract.py` (pvrgpu-platform, for `isa.md`) and
`tools/pco-isa/extract.py` (this work, for the Rust tables) both read Mesa's
`pco_isa.py`. `tools/pco-isa/crosscheck.py` compares what they produce, field
for field:

    0 disagreements about the encoding, 68 notes

Two independently written programs reading the same declarations and agreeing
on every bit piece, reserved value, form byte count, pinned value and settable
mapping is worth more than either on its own. The notes are scope, not
content, and `tools/pco-isa/README.md` says what has to close before the two
tables become one - chiefly that `usc-isa.json` does not record which reserved
fields a form pins, which a codec needs and a reference document does not.

## The IR contract (M2/M3, being agreed)

M1 deliberately has no IR: `decode` produces the encoding's own structures
(`Group`, `Region`), which are exactly reversible and nothing more. The code
generator's input is a different thing and belongs to M3; what M1 hands it is
the *target*, in two pieces:

1. `isa-coverage.md` — the operations and forms a generator must be able to
   emit to cover what the table already contains.
2. `encode_group` — the way to emit them. A generator builds a `Group` and
   encodes it; it never writes bytes itself.

What is still open, and needs agreeing with `gles2-core` before M3: whether
the generator builds `Group`s directly or through a register-allocated IR
above them, and where the phase scheduling (which operations share one group)
lives. Nothing in M1 constrains that choice.

## What M1 does not cover

The encoding only. What the registers mean, which bank the driver feeds from
where, the special registers, the DOUTI/DOUTD/DOUTT interface, the emit and
end-of-task protocol: none of that is in the encoding tables, and no round
trip can produce it. It belongs in `docs/online-shaderc/isa.md`, against the
driver's `driver/docs/gpu-recipes/shader_toolchain.md`.

`shaderc-online` also does not register a `g_shaderc` yet. Selecting it as
the back end is M3's step, when there is something behind it to compile with.
