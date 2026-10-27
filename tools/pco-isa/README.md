<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Where `src/shaderc/online/powervr/isa_tables.rs` comes from

The online shader compiler encodes and decodes PowerVR Rogue USC. The
instruction encoding is not something we invented and not something we can
guess: it is documented, as data, in Mesa's `pco` compiler
(`src/imagination/pco`, Copyright &copy; Imagination Technologies Ltd.,
MIT licence).

Two of its files are pure declarations rather than code:

* `pco_isa.py` declares every **bit set** — one encodable region of an
  instruction group: the group header, a source section, the destination
  section, the main ALU instruction word, the backend one, the bitwise one,
  the control one. Each owns named **fields**, and each field is a list of
  **pieces** (a run of bits at a byte offset), most significant piece first.
  It then declares every **bit struct**: one concrete form of that region, as
  an ordered list of field uses, each either pinned to a fixed value or left
  settable, with a byte count that the fields tile exactly.
* `pco_map.py` declares, per operation, which bit struct encodes it, in which
  pipeline phase, and which field values pin that operation down when two
  operations share one form.

So the right way to get those facts into this repository is to read the
declarations with a program, not with the eye. `extract.py` imports both
modules and writes what they declare as our own JSON; `gen_tables.py` turns
that JSON into the Rust tables. No Mesa source is vendored or copied here —
the same relationship `tools/glsl2usc.sh` already has with `pco_shc`, and the
generated file carries the MIT notice for the derived data.

## Regenerating

```sh
MESA=<mesa-26.2.1>/src/imagination/pco      # a Mesa checkout
tools/pco-isa/extract.py    "$MESA" build/isa.json
tools/pco-isa/gen_tables.py build/isa.json src/shaderc/online/powervr/isa_tables.rs
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online online::
```

The last line is the point. The round trip decodes every blob of the
PowerVR reference table into instructions and writes them back byte for byte, so a
table that lost a field, moved a bit piece or picked the wrong form shows up
as a byte that differs — a transcription defect cannot pass quietly. Check
the numbers the test prints (`N/N blobs, M instruction groups`; it was
`2042/2042 blobs, 576296 instruction groups` when this was written, and the
table has grown since) rather than only its exit status: a decoder that stopped after each blob's
first group would also exit zero.

`extract.py` needs nothing but CPython and the Mesa tree; neither script is
part of any build, and neither is needed to build or use the library.

## Two readings of the same source, and why both stay

`tools/isa-extract/pco_isa_extract.py` also reads Mesa's `pco_isa.py`, for
`docs/online-shaderc/isa.md` and its machine-readable table
`docs/online-shaderc/isa/usc-isa.json`. **The two are not folded, by the
lead's ruling, and the division is this**: the Rust tables generated here are
the shipping codec's source; `usc-isa.json` is the documentation table and
`tools/isa-extract/usc_decode.py` its verification instrument, cross-checked
against `pco_shc`'s own disassembly. Nobody should generate a codec from
`usc-isa.json`, because it does not record which *reserved* fields each
instruction form pins, and it has no `pco_map.py` half at all — no operation
names, no phase tables — and the same instruction form is a different
operation in a different phase.

They were compared before that ruling, field for field:

```sh
tools/pco-isa/crosscheck.py build/isa.json docs/online-shaderc/isa/usc-isa.json
0 disagreements about the encoding, 68 notes
```

Every bit piece, reserved value, instruction-form byte count, pinned value and
settable field mapping is identical in the two readings — two independently
written programs agreeing on all of it is better evidence for both than either
could give alone. The 68 notes are the scope differences named above: 67
reserved-field memberships and one enumeration `usc-isa.json` records that no
field on this side reaches. `crosscheck.py` is cheap to re-run whenever Mesa
moves.

## What is *not* here

The encoding only. What the registers mean, which bank the driver feeds from
where, the special registers, the DOUTI/DOUTD/DOUTT interface, the emit and
end-of-task protocol — none of that is in `pco_isa.py`, and a round trip can
never produce it. That is `docs/online-shaderc/isa.md`, written against the
driver's own `driver/docs/gpu-recipes/shader_toolchain.md`.
