<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Two vertex shaders that separate "registers" from "instruction count"

The online pair hangs the GC7000 on `dEQP-GLES2.functional.shaders.algorithm`
`hsl_to_rgb_vertex` and `rgb_to_hsl_vertex`; the reference compiler's pair passes the same
four cases on the same driver, and the `*_fragment` twins pass on both.

Our output for the wedging shader is **153 instructions, temps 41**, and five
explanations have been removed by measurement: there
are no branches in it at all, the same code as a fragment shader never wedges,
41 temps is inside the shipped vertex range, nothing over-runs its declared
temps, and the largest shipped vertex blob is 950 instructions.

These two shaders vary one axis each, past the wedging shader on that axis and
below it on the other. **Neither has any control flow.**

| | ours | reference | vs the wedging shader |
|---|---|---|---|
| `a_many_temps.vert` | 120 instructions, **temps 44** | 78, temps 40 | more temps, fewer instructions |
| `b_many_instructions.vert` | **212 instructions**, temps 5 | 50, temps 2 | far fewer temps, more instructions |

Read the result as:

- **A wedges** -> registers or thread occupancy, not the code we emit.
- **B wedges** -> instruction count or the upload path.
- **Both wedge** -> a shared cause, and the wedging shader is not special.
- **Neither wedges** -> it is the *combination*, or something neither axis
  captures, and the two real `ShcShader`s are the next evidence.

The numbers above are what our compiler reports on the host. If the board's
pair reports different ones, that mismatch is itself a finding and worth more
than the wedge result.

## D60 and D70: the pair that separates the two mechanisms

Added after two explanations of the wedge were on file at once, because
neither A nor B can tell them apart: **both A (temps 44) and B (temps 5) are
below 64**, so the driver's 6-bit `NUM_TEMPS` field carries them unchanged and
neither exercises the wrap.

| | temps our compiler reports | on the wire (`x & 0x3f`) |
|---|---|---|
| `d60_temps_low.vert` | **60** | 60 — unchanged |
| `d70_temps_high.vert` | **70** | **6** — the count wraps |

The two shaders are the same program with one axis moved. Each has **65
`vec4` definitions and 130 `+` operators**; they differ only in *when* ten of
those values are consumed. In D70 every value stays live until one final sum;
in D60 the first ten are defined and folded into the accumulator one at a
time, so each dies before the next is born. Same work, ten fewer values alive
at the peak. Neither has any control flow.

**Predictions, written before the run:**

- **The 6-bit wrap** (`regs.rs:178`, `state_3d.xml.h:201`): D60 passes, D70
  wedges. D70 is programmed 6 registers and writes 69.
- **The load-balancing band ceiling** (temps 21 for `num_out` 0|1): *both*
  wedge, since 60 and 70 are equally far above 21.
- Neither wedges: both mechanisms are wrong about this shape, and the wedge
  needs something these axes do not carry.

D60 is the arm that discriminates, and it does so against both A and the band:
60 is above the band ceiling of 21 (the band says it wedges) and below 64 (the
wrap says it passes).

**Build these at `f98bb07`. This is not a preference, it is the only build
where they mean anything.** Measured at today's HEAD, the definition-sinking
pass in `alloc.rs` collapses both of them -- D60 goes from 60 temporaries to
**6** and D70 from 70 to **6** -- because their shape (define many values,
sum them at the end) is exactly what sinking undoes: each value's first
reader is its own `add` in the sum chain, so each sinks to just before it and
nothing stays live. A pair built at HEAD tests nothing at all, and the same
collapse takes `a_many_temps.vert` from 43 to 5.

Also at `f98bb07` for the original reason: it is before either refusal landed — `MAX_TEMPS 63`
(b8b3cb1) and the driver-seam refusal (35df4ac) both reject D70 by design, and
a refused program cannot run. The instruction counts our compiler reports are
not in this table for the same reason: at today's HEAD it refuses before
emitting, so cts-runner's build at `f98bb07` is where they should be recorded.

## Stepping the temps axis: `steps/`

Built after the board refuted the truncation story. `a_many_temps` wedges at
**44** temporaries with no truncation at all, and the **reference** back end's
output for the same source wedges at **40** — so the hangs do not need a
wrapped register count, and whatever the limit is, it sits below 40. Known
points: 2, 5 and 10 pass; 40, 44 and 67 wedge.

`steps/sNN.vert` walks that gap. Each file defines NN values and reads every
one of them twice, so the measured counts are:

| file | temps | | file | temps |
|---|---|---|---|---|
| `s08.vert` | 12 | | `s24.vert` | 28 |
| `s12.vert` | 16 | | `s28.vert` | 32 |
| `s16.vert` | 20 | | `s32.vert` | 36 |
| `s20.vert` | 24 | | `s36.vert` | 40 |

`s36` is the control: it should reproduce the known wedge at 40.

**These are sink-proof, unlike `d60`/`d70`.** Each value is read once early and
once late, so the span between its two readers is what holds the register and
moving the definition cannot shorten it. Verified by compiling the set with
the sinking pass on and off: **identical temps in both, 12 through 40**. So
unlike the `d` pair, these may be built at any hash — which matters, because
the number under test is a property of the shader, not of which of our
commits produced it.

They will still be refused above 21 by the load-balancing band guard, since
that guard is the hypothesis being tested; build them where it is absent.
