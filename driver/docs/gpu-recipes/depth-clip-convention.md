<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Near/far clipping: which z convention the TA applies

GL ES 2.0 clips against `-w <= z <= w`. Vulkan and D3D clip against `0 <= z <= w`.
Which of the two the BXS-4-64's TA applies is chosen by **`CR_PPP_CTRL` bit 0**,
and we had it clear, so our near plane was Vulkan's and the clipper had to be
left off.

**Settled and implemented.** `CR_PPP_CTRL` is `0x203` (`PPP_CTRL_CLIPSPACE`) and
near/far clipping is on by default. One constant: no z remap, no shader-ABI
change, no rebuild of the offline table. What follows is how it was settled,
because the reasoning is worth keeping and the DDK capture is a good example of
evidence that looks decisive and is not.

## Measured on silicon

`driver/tools/pvrlab/user/z_probe.c` (`b607b58`, pvrgpu-platform) draws one
full-screen quad at three NDC z values with depth testing off, so only clipping
can remove it. **This first sweep ran at `CR_PPP_CTRL = 0x202`**, the value we
had then -- which is what the last column is reading:

| `PVR_BACKEND_OPT_DEPTH_CLIP` | TA clip mode | z=-0.5 | z=+0.5 | z=+2.5 | reading |
|---|---|---|---|---|---|
| 0 | `NONE` | drawn | drawn | drawn | nothing clips |
| 1 | `FRONT_REAR` | **CLIPPED** | drawn | CLIPPED | `0 <= z <= w` |
| 2 | `FRONT_REAR_DEPTH` | drawn | drawn | drawn | clips nothing at all |

`z=+2.5` is outside both volumes and `FRONT_REAR` removes it, so that mode really
is clipping. `z=-0.5` is inside GL's volume and outside D3D's, and `FRONT_REAR`
removes it. Hence: **at 0x202 the TA clips `0 <= z <= w`** -- and the second
sweep below shows that is the register bit, not the hardware's only convention.

`FRONT_REAR_DEPTH` (mode 2) is not a stricter `FRONT_REAR`; it clips nothing on
its own and needs something else programmed. Do not default to it.

## What the DDK capture showed (conclusions only)

A captured DDK GL ES triangle (since removed from the tree) set `CR_PPP_CTRL`
bit 0 where we had it clear. The decoded words are not reproduced here.

**1. `CR_PPP_CTRL` bit 0 was the lead.** The DDK, an OpenGL ES driver, sets
bit 0 (`opengl`); Mesa's `pvr_arch_job_render.c`, a **Vulkan** driver, leaves it
clear. Bit 1 (`wclampen`) is set in both. Bit 9 (`fixed_point_format`) is
set by Mesa and by us, and was clear in the DDK's value.

| | bit0 `opengl` | bit1 `wclampen` | bit9 `fixed_point_format` |
|---|---|---|---|
| DDK (an OpenGL ES driver) | **1** | 1 | 0 |
| Mesa (a **Vulkan** driver) | 0 | 1 | 1 |
| us, `PPP_CTRL_CLIPSPACE` **then** | **0** | 1 | 1 |
| us, `PPP_CTRL_CLIPSPACE` **now** | **1** | 1 | 1 |

We took Mesa's value because our VS emits Mesa's clip-space fixed-point
positions, and inherited bit 0 = 0 with it. Vulkan is exactly the API whose
convention we observe. That a bit named `opengl` is set by the GL driver and
clear in the Vulkan one, and that we see the Vulkan convention, is a coherent
hypothesis that the bit selects the clip convention -- but at this point it was
only that: the name comes from our reconstruction, not from a register spec,
and nothing in the capture isolates the bit. The sweep below is what turned it
from a good guess into a measurement.

**2. The DDK did not use the TA clipper for this draw.** Its PPP state block
had the clip mode off (`NO_FRONT_OR_REAR`) and pretransform on.

**3. The capture cannot answer the question, and that is worth stating.** Its
vertices were pre-transformed screen-space positions with `w = 1.0` and
`z = 1.0` on every vertex, inside both volumes. With clipping off and an
identity transform, that triangle exercises no clipping at all. So the capture
shows no z remap **because it would show none either way**. It is not evidence
that the DDK has no remap; it is a draw that cannot tell us. Answering it from a
capture needs a DDK frame with a real perspective projection, which we do not
have.

## The experiment, and what it answered

`PVR_BACKEND_OPT_PPP_CTRL` (a test knob: it takes effect in a `test-knobs`
build, as gl_test's) overrides the register at run time and `z_probe`
links `libyakogl.a` statically, so the sweep needed no shader and no rebuild:
`ppp_ctrl` in {`0x202`, `0x203`} against depth clip in {0, 1}. `0x203` is our
value **plus** bit 0, keeping `fixed_point_format` (bit 9) set, because our VS
emits Mesa's coordinate format -- which is why the DDK's bare `0x3` was never
the value to try.

Measured: at **`0x203` with FRONT_REAR the quad at NDC z -0.5 is drawn and the
one beyond the far plane is still clipped**. That is GL's volume. Bit 0 is the
convention switch.

## What is programmed now

- `PPP_CTRL_CLIPSPACE = 0x203` -- `opengl` | `wclampen` | `fixed_point_format`.
- `PVR_BACKEND_OPT_DEPTH_CLIP` defaults to `FRONT_REAR`, so an application that
  asks for nothing gets GL's clipping. The knob keeps the values `z_probe` and
  `gl_test` case 62 pass (1 FRONT_REAR, 2 FRONT_REAR_DEPTH); what changed is
  that 0 now selects the default instead of "off", and switching clipping off
  for bring-up takes `PVR_DEPTH_CLIP_OFF` (3).
- `FRONT_REAR_DEPTH` (2) still clips nothing on its own. It is a bring-up
  value, not an alternative.

**One thing z_probe cannot tell us**, and it needs a board run rather than an
argument: whether anything *else* depends on bit 0. It is one bit in a register
whose other bits select coordinate formats, and the probe draws one full-screen
quad. The host tests pin the words we emit; they cannot see the silicon. So the
`gl_test` regression set on the board is what says this change costs nothing
elsewhere, and until that has run this is implemented-and-unverified rather than
done.

## The fallback, now unused

The standard remap -- `z' = (z + w) * 0.5` in the hoisted `gl_Position`
epilogue, the backend viewport z transform mapping `[0,1]` instead of
`[-1,1]`, the hand-written passthrough quads moved inside `[0,w]`, and the
offline table rebuilt -- is what this would have cost if bit 0 had not been the
answer. It is a shader-ABI change across every shader in the table, which is
why the one-register experiment came first. Keeping the note here so that the
next person who suspects the z convention knows the cheap question to ask
before reaching for the expensive answer.
