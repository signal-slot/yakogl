<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000: the CPU depth resolve works on whole rows

`ds_cache::resolve_depth` (`src/vivante/ds_cache.rs`) turns the D24S8 depth
surface of a depth texture into the texture's floats on the CPU. glmark2
`shadow` and `refract` do it once a frame (`docs/notes/imx8mp-glmark2-cpu.md`,
"Fix items, in order", item 2).

## What was wrong

The surface is row-linear at its row stride (not tiled: the doc comment of
`resolve_depth` has the 2026-09-30 board measurement). The old loop called
`ds_retention::write_depth_texels` with a closure that, for every texel,
compared `x`/`y` with the surface, did `px[o..o + 4].try_into().expect(..)`
(a checked slice) and returned an `Option<f32>`. The conversion itself is one
shift and one divide.

## The change

One loop per row on plain slices: `src.chunks_exact(4)` zipped with the
destination row slice. No closure, no checked index per texel. The columns are
`min(w, stride / 4)` and the rows `min(h, rows)`: a texel past the surface is
left as it was, as before. The clear-value path (`fill`) is unchanged.

## Bit-exact

`tests/gc_depth_resolve.rs` fills a surface made by the real
`ds_cache::select` with a fixed xorshift pattern (plus the extremes
`0x000000ff`, `0xffffff00`, `0x80000000`) and checks every float's bits
against `((word >> 8) as f64 / 16_777_215.0) as f32`. It covers six
(w, h, pitch) cases, among them a pitch with padding, a request larger than
the surface and 1 x 1. Written and run against the old loop first (passes),
then against the new one (passes): the output is bit-identical. The 90 x 70
surface in the test is 128 words wide and 128 rows tall.

## Host numbers

Micro-benchmark: `time_one_1024_square_resolve` in the same file, one
1024 x 1024 resolve, 40 runs, release build, x86-64 host, nothing else running:

| | best | mean |
|---|---|---|
| per-texel closure (before) | 1.100 ms | 1.25 - 1.33 ms |
| per row, chunks_exact (after) | 0.556 ms | 0.655 ms |

About 2x. What is left is the f64 divide per texel (`unorm24_depth`, in
`src/vivante/ds_retention.rs`, which this change does not touch; a reciprocal
multiply was not tried, it is not bit-exact by construction and would need an
exhaustive check over the 2^24 values).

The 2.1 ms a frame per scene that the note above measured is the host harness
number for the old loop on a different machine and build; the 1.1 ms is
this micro-benchmark's own.

**`tools/hostharness/glmark2` (`GM2_PROFILE=1`, vivante) for shadow and
refract.** "Before" is `6e0c2263`, the parent of the change, "after" is
`0d21ab47`, the change itself; each is a separate checkout, built and run on
the same x86-64 host, one run each, 20 frames per entry of which 19 are
counted (`GM2_FRAMES=20`). Per checkout, with `$GLMARK2_SRC` glmark2 at the
commit `tools/hostharness/glmark2/README.md` names:

```sh
GM2_PROFILE=1 GM2_WRITE=0 GM2_FRAMES=20 GM2_ENTRIES='^(shadow|refract)$' \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/gm-<tag> vivante
```

`profile-vivante.md` has no row of its own for `resolve_depth`: it runs inside
the `glDrawArrays` that samples the depth texture, so that row is the figure
for the resolve (with the draws of the frame: 3 calls a frame in shadow, 2 in
refract). The total is the "in GL and EGL calls" figure of the scene. All in
wall ms a frame:

| scene | before `6e0c2263`: `glDrawArrays` / total GL | after `0d21ab47`: `glDrawArrays` / total GL |
|---|---|---|
| shadow | 6.805 / 6.838 | 3.088 / 3.121 |
| refract | 4.851 / 38.826 | 2.382 / 28.901 |

`glDrawArrays` (resolve included) is 3.7 ms a frame less in shadow and 2.5 ms
less in refract. Refract's total GL time is dominated by `glGenerateMipmap`
(the CPU box filter of its chain, which this change does not touch and which
moves between runs: 33.9 ms before, 26.5 ms after), so its 9.9 ms drop in
total is not all this change. One run each: a reading, not a measurement
series. On the board (i.MX8MP, main `c5f10ea9`, `tools/cts/RESULTS.md`
"Common set") shadow stays at 2 FPS and refract at 1 FPS: the integer FPS of
the SNVS record cannot show a change this small.

## The real fix (not done)

No CPU read at all: sample the D24S8 surface with the texture unit.
`src/gl/texture.rs` (around line 680) says reading a depth surface through the
TPU was "not tried on either board". It needs a board check first that the
GC7000's texture unit reads the depth surface (format, tiling, the stencil
byte) before any code relies on it. Until then the CPU resolve stays, and this
change only makes it cheaper.
