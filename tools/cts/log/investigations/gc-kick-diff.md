<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# KICK_DIFF: the second read-back for i.MX8MP gl_test #9 and #25

**Status (2026-10-12): instrument on main, not yet called.** Re-landed from
integration (`af9767d5b`, ccfarm `01a12497-6bbf-7511-9885-19689ca6bd93`).
Nothing calls `gc_kick_diff_after_fence()` yet: the gl_test call site and
the record words are still owed ("Call site" below), and no board has run
it. The record `imx8mp-es2-r1.md` cited below is re-landed separately under
`tools/cts/log/board-sessions/`.

Item `gc-flake-instrument` of plan #20. Host only: no board was run. The code
is in `src/vivante/kick.rs` (module `diff`), the tests in
`tests/gc_kick_diff.rs`. The idea comes from
`tools/cts/log/investigations/gc-case10-flake.md` ("the late-read instrument"), which
this file does not change.

## What it answers

gl_test case 10 (#9) and case 2 (#25) lose the pixels of a tiny draw that
follows a clear in one kick. The lost pixels read back as the clear colour.
Two stories fit:

- **late write**: the fence fired, but the pixel engine's write-back reached
  memory afterwards;
- **lost or wrong write**: it never arrives.

The instrument reads the render target twice after the fence: once at once,
once after a delay, behind a load barrier (`dmb ishld` on aarch64).

| first read vs second read | reading |
|---|---|
| the second differs (verdict `Late`) | the first read was too early. The event fired before the write-back landed (or the CPU read passed the fence). |
| equal (verdict `Stable`) | the target did not change in the delay. It was right at the fence, or the write was lost. The instrument alone cannot say which: use the case's own check (the missing pixels are still the clear colour in a lost write). |

A `Late` verdict on a failing run is the finding. A failing run with `Stable`
means the PE dropped the write (or wrote something else): then the K1 knob
below is the next step, not a longer delay.

## How to enable it

It is a test knob. It exists only with the crate's `test-knobs` feature, as
`gc_debug_msaa` does, and a product image never has it (`tools/check.sh`,
"no test knob in the artefact"). Without the feature the code is not in the
build, so a normal build is unchanged. With the feature it is still off until
a harness arms it.

C entry points (all in `yakogl` with `test-knobs`, GC7000 only):

```c
void gc_kick_diff_enable(const uint32_t *target_cpu, uint32_t words,
                         uint32_t pitch_words, uint32_t delay_us,
                         void (*delay)(uint32_t us));  /* NULL ptr disarms */
int  gc_kick_diff_after_fence(void);   /* 0 off, 1 stable, 2 late */
void gc_kick_diff_stats(uint32_t out[3]); /* checks, late, most words changed */
```

- `target_cpu` is the CPU view of the colour target, `words` its size in
  32-bit words (at most 4096, the 64x64 RGBA8 surface gl_test uses; more is
  cut), `pitch_words` the row length (64 for that surface).
- `delay` is the harness's busy wait. The OSAL has no sleep op, so the knob
  takes the routine. 10000 us is the 10 ms the older notes ask for. With a
  NULL `delay` the second read follows at once (still tests a CPU-side
  reordering).
- Call `gc_kick_diff_after_fence()` right after `glFinish()` and before the
  case's `glReadPixels`. Do it once per check. Arming resets the counters.

Host build and test:

```sh
cargo test --no-default-features --features host-test,gpu-vivante,test-knobs --test gc_kick_diff
```

**Call site.** This item may not edit `src/vivante/backend.rs` (the K1 item
holds it), so the backend does not call the instrument. The harness does
(gl_test, after `glFinish`). A later item that owns `backend.rs` can call
`gc_kick_diff_after_fence()` once in `frame_submit` after the fence is seen,
if the harness call turns out too late to matter. `cmd.rs` is not changed:
the instrument only reads memory, it adds no command to the stream.

A board run additionally needs the gl_test source to call these three
functions and put `gc_kick_diff_stats` into its counters record. That is a
change in `driver/` and belongs to `imx8mp-gltest-r2`.

## How to read its output

Two places, both existing:

1. **The OSAL log** (`glog!`, so the board's console or panel):

   ```
   yakogl/vivante: KICK_DIFF stable (4096 words, second read 10000 us later)
   yakogl/vivante: KICK_DIFF late: 4 of 4096 words changed after the fence, rows 4, first word 1233 0xff000000 -> 0xff0000ff, last 1553, second read 10000 us later
   ```

   The `stable` line is printed once (the first check). Every `late` line is
   printed. Fields: how many words changed, in how many rows (of
   `pitch_words`), the first changed word index and its two values, and the
   last changed index. The 1 px and 3 px points of case 10 frame 5 should
   show as 4 changed rows if the late-write story is right (`tools/cts/STATUS.md` at the time, line 110).

2. **The counters** from `gc_kick_diff_stats`: checks made, checks with a late
   write, the most words one check saw change. The harness puts them in its
   record. The record kind 19 (KICK_DIFF) is reserved in
   `driver/rust/src/board/record.rs` but not defined; this item does not
   define it (that file is not in its scope). Until it is, put the three
   words in the sweep's spare words.

## How it combines with K1

K1 (v3d-null-cpu item, `src/vivante/backend.rs` `emit_flushes`) removes the
per-draw COLOR|DEPTH flush when the previous draw wrote the same targets,
as etnaviv does. KICK_DIFF is independent of it: it reads memory after the
fence and does not look at the command stream. So the two cross as a 2x2
(interleaved boots, as in gc-case10-flake.md):

| arm | K1 | KICK_DIFF | expect if the flush race (a1) is the cause |
|---|---|---|---|
| base | off | off | #9 fails about 5/10 (2 of 3 at `334d4e65`) |
| base+kd | off | on | same rate; each failing run says `late` or `stable` |
| k1 | on | off | 0/10 |
| k1+kd | on | on | 0/10, and no `late` line |

Reading:

- base+kd failing **with** `late`: the fence is early; K1 may still fix it by
  removing the write that races, but the OSAL epilogue (the FE->PE stall and
  the event) is also a suspect. Add the `dmb` and the longer wait to the OSAL.
- base+kd failing with `stable`: the write was dropped. K1 is the test. If
  k1 stays at 0/10, a1 is confirmed.
- k1+kd failing: K1 is not the cause. If those runs say `late`, go to the
  epilogue. If `stable`, go to the clear (K2, RS fill).
- Running the instrument must not move the rate: it adds a 10 ms wait after
  `glFinish` only. Compare base against base+kd; if they differ much, the
  wait itself hides the fault, and a NULL `delay` arm shows that.

## Candidate commits between `b20e4d11` and `334d4e65`

The data: #9 failed first in 2 of 3 runs at `334d4e65` and in 0 of 3 at
`b20e4d11` (`tools/cts/log/board-sessions/imx8mp-es2-r1.md`). Three runs are few:
0 of 3 against 2 of 3 is not significant (Fisher's exact test, two-sided,
p = 0.4), so this may be no regression at all.
The range holds 64 non-merge commits; these touch code that the i.MX8MP build
runs:

`git log b20e4d11..334d4e65 -- src/vivante driver/src/vivante` lists one
commit:

| commit | what | verdict |
|---|---|---|
| `c86adc9` | clippy `needless_range_loop` in `src/vivante/backend.rs` (`hw_targets`, `emit_streams`, `gc_init`, `gc_shutdown`), `backend_tests.rs`, `shader.rs` | An index loop became an iterator. The loop bounds are kept (`take(n)`, `skip(1)`); read by hand, no behaviour change. In `emit_streams` the three loops still run in the same order, so the stream is the same. A candidate only by path. |

`driver/src/vivante` has no commit in the range.

Code the GC7000 build shares with the other GPUs also changed (`git diff
--stat b20e4d11 334d4e65 -- src driver`):

| commit | what | verdict |
|---|---|---|
| `cc78859` | "pvr: npot 2d textures get real mip chains": `src/gl/draw.rs` (+104), `framebuffer.rs`, `state.rs`, `context.rs` (+56), `texture.rs` (+323), `abi_layout.rs`, `gl_context.h` | The largest change in shared GL code, so it is the best candidate for a changed draw or allocation order on any GPU. Its title says PowerVR, but `draw.rs` and `framebuffer.rs` run on the GC7000 too. It also changes `GlContext` (the C ABI header), so the gl_test image changes size and the pool placement after the surface may move. The colour target is allocated before any texture, so its address does not (F2 of gc-case10-flake.md). Not read in full here. |
| `7bc37ed` (merge) | `driver/rust/src/lib.rs` +3, `src/lib.rs` +4 | Two small additions; not a draw path. |
| `3c4349c` (merge of `c86adc9`'s branch) | the same clippy rewrite in `src/gl/draw.rs`, `framebuffer.rs`, `program.rs`, `vao.rs`, `src/testutil.rs` | Same as `c86adc9`. |
| `ccc410e`, `b528cd4` | clippy allow-lists | No code. |

Everything else in the range is documentation, tests, CTS scripts (the board
run's tooling) or ccfarm files. One remark on the tooling: the image builder
(`tools/cts/imx8mp-deqp-build.sh`) changed no line in the range
(`git diff --stat` shows no `tools/cts/*.sh`), so the two board sessions used
the same build recipe.

To bisect, build gl_test at `cc78859^` and at `cc78859` (and at `c86adc9^`
and `c86adc9`), 10 interleaved boots each, instrument on. With a rate of
about 5/10, 10 boots separate 0/10 from 5/10 at roughly the 0.2 % level the
older recipe quotes. Do not bisect on 3 boots.

## What was checked, and what was not

- Host: `tests/gc_kick_diff.rs`, 7 tests: equal reads, a late pixel with its
  values, row counting, zero pitch, unequal lengths, a write that never
  lands (stable), and the armed path with a fake delay routine that lands a
  write during the wait (verdict 2, then 1; counters; disarm).
- `tools/check.sh --quick`: see the report of this item for the output and
  the base comparison.
- Not checked: any board behaviour. The instrument has not read a real GC7000
  target. The `dmb ishld` is aarch64-only and is not exercised on the host.
  The claim that the older notes make, that a 10 ms wait is long enough for a
  late write, is untested.
