<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# KICK_DIFF: second read-back for i.MX8MP gl_test #9 and #25

Item `gc-flake-instrument` of plan #20. Host only: no board was run.

The full record (call site, board recipe, record page layout, what was
checked) is `tools/cts/log/investigations/gc-kick-diff.md`. This file is the
short guide: how to enable it, how to read it, how it combines with K1, and
the candidate commits.

The code is `src/vivante/kick.rs`, module `diff`. The host tests are
`tests/gc_kick_diff.rs`. `src/vivante/cmd.rs` is not changed: the
instrument only reads memory and adds nothing to the command stream. Nothing
here edits `backend.rs`, `backend_tests.rs` or `src/osal/**`.

## What it answers

Case 10 (#9) and case 2 (#25) lose the pixels of a tiny draw that follows a
clear in one kick. Two stories fit:

- **late write**: the fence fired, then the pixel engine wrote.
- **lost or wrong write**: the write never lands.

The instrument reads the render target right after the fence, waits, and reads
it again. It compares the two reads.

## How to enable it

It is off by default, in two layers:

1. Cargo feature `test-knobs`. Without it, the code is not in the build, so a
   normal build and its tests do not change.
2. A runtime arm. With the feature on, nothing happens until the harness calls
   `gc_kick_diff_enable(...)` or `gc_kick_diff_arm_target(delay_us, delay)`.

```sh
cargo test --no-default-features --features host-test,gpu-vivante,test-knobs --test gc_kick_diff
```

The harness calls `gc_kick_diff_after_fence()` after `glFinish()` and before
its own `glReadPixels`. Arming resets the counters. `gc_kick_diff_stats(out)`
returns checks, late checks and the most words one check saw change.

## How to read the output

The result goes to the OSAL log (`glog!`) and to the counters. gl_test also
keeps it in a PERF record page (see the full record).

| output | meaning |
|---|---|
| `KICK_DIFF late: N of W words changed after the fence ...` | The first read was too early. The write landed after the fence. Printed for every late check. |
| `KICK_DIFF stable (...)` | Both reads are equal. The target was right, or the write was lost. Printed once. The case's own pixel check tells which. |
| return value 0 / 1 / 2 | off / stable / late |

A failing run with `late` points at the fence or the epilogue. A failing run
with `stable` means the write was dropped (or the delay was too short).

## How it combines with K1

K1 is the etnaviv flush rule in `emit_flushes`, added by the `v3d-null-cpu`
item in `src/vivante/backend.rs`. It is not on this tree yet. KICK_DIFF does
not depend on it: it reads memory after the fence and never looks at the
stream. Cross them as a 2x2, with interleaved boots:

| arm | K1 | KICK_DIFF | expect if the flush race is the cause |
|---|---|---|---|
| base | off | off | #9 fails about 5 of 10 |
| base+kd | off | on | same rate; each failing run says `late` or `stable` |
| k1 | on | off | 0 of 10 |
| k1+kd | on | on | 0 of 10, no `late` line |

If base+kd fails with `stable` and k1 is 0 of 10, the flush race is
confirmed. If k1+kd still fails, K1 is not the cause: `late` points at the
epilogue, `stable` at the clear. If base and base+kd rates differ much, the
wait hides the fault; run a NULL-delay arm.

## Candidate commits, `b20e4d11..334d4e65`

Data: #9 failed first in 2 of 3 runs at `334d4e65`, 0 of 3 at `b20e4d11`
(`tools/cts/log/board-sessions/imx8mp-es2-r1.md`). Fisher's exact test, two
sided, gives p = 0.4. This may not be a regression at all.

`git log --no-merges b20e4d11..334d4e65 -- src/vivante driver/src/vivante`
lists one commit:

| commit | what | verdict |
|---|---|---|
| `c86adc9` | clippy `needless_range_loop` rewrite in `src/vivante` (`backend.rs`, `backend_tests.rs`, `shader.rs`) and shared GL code | Index loops became iterators with the same bounds. Read by hand: no behaviour change. A candidate only by path. |

`driver/src/vivante` has no commit in the range. Shared GL code that the GC7000
build also runs changed in `cc78859` ("pvr: npot 2d textures get real mip
chains": `src/gl/draw.rs`, `framebuffer.rs`, `state.rs`, `context.rs`,
`texture.rs`, and the `GlContext` C header). It is the best candidate for a
changed draw or allocation order, but it is not in `src/vivante`.

To bisect, build gl_test at `cc78859^` and `cc78859` (and `c86adc9^`,
`c86adc9`), 10 interleaved boots each, with the instrument on. Do not bisect
on 3 boots.
