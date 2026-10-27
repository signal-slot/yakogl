<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# glmark2 `build:use-vbo=false`: client arrays copied by rows

`build:use-vbo=false` draws from client arrays. On the Pi 4 it runs at 6.39 ms
a frame with 2.73 ms of GL-call time, against 3.62 ms and 0.02 ms for
`use-vbo=true` (docs/notes/pi4-glmark2-cpu.md, "On the board"). On the GC7000
host profile the cost is `attrib_row`, `convert_attrib` and `memmove`
(docs/notes/imx8mp-glmark2-cpu.md, "4. Smaller").

## The change

Commits: before `80ddfaa37b9c` (main), after the commit that adds this file
(`src/gl/draw.rs`).

`convert_attrib` always wrote packed float32 rows, one component at a time
through `attrib_row`/`attrib_component`. A GL_FLOAT array (not an integer
one) that supplies every component the input reads already is that layout,
so it now copies the first `out_comps * 4` bytes of each element: one
`copy_nonoverlapping` of the whole range when the stride equals the row, one
per vertex otherwise. The checks are made once before the loop. The arena
copy stays. Everything else keeps the per-element path: normalized or
unnormalized ubyte/byte/short/int/fixed/half, packed 2_10_10_10, an array
with fewer components than the input (the missing ones read 0, 0, 0, 1), an
array with no CPU view.

So of glmark2's client arrays only the float ones gain; glmark2's `build`
scene sends float positions and normals, which is what the table below
measures.

## Pinned by `tests/gl_client_array_copy.rs`

Arena bytes of the fast path equal the slow path's (`CLIENT_COPY_FAST`
switch) and the converted rows, for float tight, strided, unaligned stride,
interleaved at offset 0 and 12, a 4-component array into a vec3 input; ubyte
normalized and short are converted (`CLIENT_COPY_FAST_ARRAYS` does not move),
as is a 2-component float array. The arena keeps its copy after the client
memory is overwritten. The test runs in every `host-test,gpu-powervr,shaderc-online`
row of `tools/check.sh` (the row runs all `tests/*.rs`), so it needs no list
entry.

Its timing test (`--nocapture`, not asserted; 61 vertices a draw, whole GL
call including draw setup), run on this change: tightly packed 0.62-0.70 us
fast against 1.75-1.86 us slow; stride 32 0.61-0.72 us against 1.50-1.58 us.

## Host before/after table, `build:use-vbo=false`

Method: glmark2 `22c527cb0556f3a1ac4445aaa52cc532760928d5`, 19 counted
frames, 10 repeats per side, the sides interleaved (before, after, before,
...). Both sides are the same tree: the "before" runs have `src/gl/draw.rs` of
`80ddfaa37b9c`, the "after" runs that of this change; nothing else differs.
Each repeat, both GPUs:

```sh
GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=20 GM2_ENTRIES='^build:use-vbo=false$' \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/gca-out v3d vivante
```

| GPU | before `80ddfaa37b9c` min wall ms a frame | after (this change) min wall ms a frame | GL-call ms a frame before / after (min) | before range | after range |
|---|---|---|---|---|---|
| v3d | 0.452 | 0.053 | 0.442 / 0.043 | 0.452-0.697 | 0.053-0.061 |
| vivante | 0.465 | 0.043 | 0.455 / 0.034 | 0.465-0.576 | 0.043-0.052 |

Minimum over the repeats, per FINDINGS-variance.md (wall ms a frame of the
`profile-<gpu>.md` entry; the GL column is its "in GL and EGL calls"). On both
GPUs the `glDrawArrays` call went from about 0.44 ms to about 0.04 ms a
frame, a 9 to 10 times drop. These are host milliseconds, not the boards'.

## Hand-over

* The host table is filled; the boards are not measured.
* Board sessions: compare `build:use-vbo=false` GL-call time against the
  2.73 ms (Pi 4) before-figure; the CPU there reads the client memory
  cached, so the gain should be at least the host's. Expect no change for
  the ubyte/short arrays.
* If the host shows `convert_attrib` still ranked, look at the remaining
  per-draw costs: `staging_reserve` and the cache flush of the scratch.
* No `tools/check.sh` row is needed.
