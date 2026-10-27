<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Slint on the Pi 4: the 312b / 0xe7113010 picture (stage-2 gap item 8)

Question: the Pi 4 Slint run of `d64e995b` (board session `pi4-common-int`,
merged as `79c73ea2`) gave p2 `0862`, crc16 `312b` and window fingerprint
`0xe7113010`; the reference is p2 `0842`, crc16 `c592`, `0xfaa46995`. Is that
a correct new reference or a regression?

## Decision

**Neither. The reference stays (`c592`, `0xfaa46995`); there is nothing to
fix on main.** The Pi 4 gives the reference again on current main, and the
`312b` picture is not reproduced. No code and no reference file changed.

What is not settled, and cannot be on the host: which commit moved the
picture to `312b` and which moved it back (see "Why no commit is named").

## Evidence

Board records (all Pi 4, V3D, same 1024x600 window, two frames, t = 0; the
frame depends on the time stamp only, so the picture is deterministic):

| Run | Tree | SO record (w1..w3) | SV record | Window crc32 |
|---|---|---|---|---|
| baseline (since `8abeaabb`, shared with AM62P and i.MX8MP) | | `08420862 00210842 c5920883` | same | `0xfaa46995` |
| `pi4-common-int`, 2026-10-14 | `d64e995b` | `08420862 00210862 312b0883` | identical to SO | `0xe7113010` |
| `pi4-common-int`, 2026-10-17 | `c854e0ae` | `08420862 00210842 c5920883` | identical to SO | `0xfaa46995` |

- Two separate boots (SO, SV) agreed at `d64e995b`, and two agreed again at
  `c854e0ae`. So the `312b` picture was a deterministic result of that tree,
  not noise of one boot; and it is gone on `c854e0ae`.
- The other four points did not move (p0 `0862`, p1 `0842`, p3 `0021`, p4
  `0883`). Only p2, "face-right" (810, 330), moved. Its reference is
  (8, 11, 17). `rgb565` truncates (`driver/rust/src/board/record.rs:609`):
  green `0842` -> 6-bit 2 (8..11), `0862` -> 6-bit 3 (12..15). The check
  passed at tolerance 1, so p2's green was exactly **12** against the
  reference's 11: one 8-bit level on one channel of one probe. The crc32
  changes with any one pixel, so `0xe7113010` says only that some pixels
  differ by an unknown amount; the probes do not say where else.
- The AM62P and the i.MX8MP stayed at the reference (`c592`) the whole
  time, so the reference was not wrong for the other two GPUs.

Judging the picture: a one-level step in one channel at one probe, in a
smooth dark gradient, with no other probe moved, looks like a rounding
difference. Neither candidate in the gap note explains it by what it changes:

- **npot** (the NPOT sampling and level fixes of 2026-10-13/14, e.g.
  `15cdd554`, `825571ea`): they change how a non-power-of-two texture is
  sampled (mip, wrap). Slint's textures are the glyph atlas and image
  textures; a changed sampling would move text and image pixels, not one
  gradient probe by one level.
- **femtovg pin**: pins the renderer's source
  (`driver/samples/zephyr/nostd/README.md`); the replay of `c854e0ae`
  (`tools/slint/replay/README.md`) shows the same draws
  per frame (475 first, 242-248 after) as the replay of `5f8fff01`, so the
  draw stream did not change.

A one-level gradient step is the kind of change a blend, an interpolation or
a store-path rounding makes. The commit `c6f17516` (V3D renders a target to a
second UIF image and samples it as it is) is in `c854e0ae` and not in
`d64e995b`; it is the largest V3D render-path change in that window and the
first suspect for the move back. This is a guess, not a measurement.

## Why no commit is named

- The host has no V3D rasteriser. The replay
  (`tools/slint/replay/`) runs the sample's GL stream
  over the real library with "nothing rendered": it sees GL errors, refused
  draws and draw counts, not pixels. `tools/cts/testdata/` holds no Slint
  picture. `common-tally.py` only reads the probes and crc16 from a board
  record. So the host cannot reproduce `312b`, and a host bisect of the
  picture is impossible. I did not build the replay: it could not show a
  pixel anyway.
- A board bisect needs a Pi 4 image per commit between `8abeaabb` (last
  known `c592`) and `d64e995b` (`312b`), and then between `d64e995b` and
  `c854e0ae` (`c592` again): about 20 images of several minutes each, for a
  one-level difference that the check accepts. Not worth it unless `312b`
  returns.
- The window of the move to `312b` is `8abeaabb..d64e995b` (GL core texture,
  state and blit changes and the V3D occlusion, cube and tf-api work of
  2026-10-09 to 10-14). The window of the move back is `d64e995b..c854e0ae`
  (`c6f17516`, `ee66f914`, `2c951bcf`, `0247ac66` and others).
  `git log --oneline <a>..<b> -- src/v3d src/gl` lists them.

## The bisect over `c854e0ae..499b946e` (2026-10-18)

`312b` came back at `499b946e` and at `33a0f264`, so the bisect between the
last `c592` (`c854e0ae`) and the first new `312b` (`499b946e`) was due.

**Result: no commit is named, because no commit in the window changes what
the SO image draws. The picture changes between board sessions, not with the
source: intermittent across sessions, stable within one.** `c6f17516` is not
in the window (it is in `d64e995b..c854e0ae`), so it cannot be the cause of
this return.

### The window has one code change for the Pi 4 SO image, and it draws nothing different

`git log --oneline c854e0ae..499b946e` has 21 commits. Only these change
files a Pi 4 image is built from (`git log --stat`):

| Commit | What it changes | Reaches the Pi 4 SO image? |
|---|---|---|
| `7bf47fe9` | `tests/v3d_online_corpus.rs`, a list under `tools/shaders/` | no (host test) |
| `dd28726c` | `driver/zephyr/lib/board/imx8mp/`, a Kconfig option that `depends on YAKOGL_BOARD_IMX8MP` | no |
| `f511a720`, `f1367c9a` | `driver/samples/zephyr/host-replay/`, replay READMEs | no (host replay) |
| `8662e5b2` | `tools/board.sh`: refuses a bad `BOARD_HDMI_TIMINGS` | no (host side) |
| `1eb9783d` | EGL double buffer (`src/gl/egl.rs`, the OSAL `present` op, the samples ask for `EGL_SINGLE_BUFFER`) | yes, see below |

All the others are `docs/` only. So for this image `c854e0ae` builds the same
code as `d4a8aee4` (the parent of `1eb9783d`), and `1eb9783d` the same as
`499b946e`: the bisect is the one step `d4a8aee4` -> `1eb9783d`.

What `1eb9783d` changes for the Slint window: `examples/slint/src/main.c`
now passes `EGL_RENDER_BUFFER, EGL_SINGLE_BUFFER` to
`eglCreateWindowSurface`. In `src/gl/egl.rs`, `create_window_surface` then
sets `back = false`. `eglSwapBuffers` computes
`complete = window && !back && !pipelined`, which is `true` as before the
commit, so the swap is `backend::flush()` then `backend::finish()`, as
before. `window_buffers_ready` and `swap_window_buffers` (the two extra
buffers, `osal::present`) only run when `back` is set. `render_buffer_of`
still gives `EGL_SINGLE_BUFFER`. The OSAL gains a `present` pointer in
`osal_zephyr_ops` (`src/osal/zephyr_v3d.c`), and `abi_layout.rs` moves the
offsets of `lock`, `unlock` and `thread_id`, nothing else. So the commands
of every Slint frame are the same before and after `1eb9783d`: the commit
cannot move one level of green at p2.

### SO runs per commit

Pi 4, V3D, the SO row (1024x600 window, copy path), two frames at t = 0.
"Runs" counts boots; SV gave the same record as SO in every session that
ran it.

| Commit | Date | Session | SO record (w1..w3) | crc16 | crc32 | Runs |
|---|---|---|---|---|---|---|
| `d64e995b` | 2026-10-14 | `pi4-common-int` (`fca07828`) | `08420862 00210862 312b0883` | `312b` | `0xe7113010` | SO 1, SV 1 |
| `c854e0ae` | 2026-10-17 | Pi 4 common set (`c6c6b673`) | `08420862 00210842 c5920883` | `c592` | `0xfaa46995` | SO 1, SV 1 |
| `499b946e` | 2026-10-18 | `pi4-db-baseline` (`e90dd520`) | `08420862 00210862 312b0883` | `312b` | not read | SO 1, SV 1 |
| `33a0f264` | 2026-10-18 | `pi4-gm-swap-board` (`a8fdc9f0`) | `08420862 00210862 312b0883` | `312b` | `0xe7113010` | SO 1, SV 1 |
| `5ba988ee` | 2026-10-18 | this bisect | `08420862 00210842 c5920883` | `c592` | `0xfaa46995` | SO 3 (2 builds) |

Counts of `312b`: `d64e995b` 2 of 2, `c854e0ae` 0 of 2, `499b946e` 2 of 2,
`33a0f264` 2 of 2, `5ba988ee` 0 of 3.

The console of the three runs at `5ba988ee`
(`records/pi4/slint-bisect/so-5ba988ee-console.txt`):

```
sample: build tree=5ba988ee table=c95547a1
slint: point face-right  (810,330) = 8,11,17,255: AM62P 8,11,17 (diff 0)
slint: window 1024x600: 0 sentinel pixels left, 0 with alpha != 255, guard rows 0 of 8192 changed, crc32 0xfaa46995
slint: check PASS (tolerance 1 per channel)
yk-record: 594b0f08 faa46995 00000002 04000258
yk-record: 594b0601 08420862 00210842 c5920883
```

The same lines came three times: image `9b6391b1` (built by
`tools/cts/board-stage.sh pi4 common`), image `fcd95dcf` (the same tree built
in another directory: 4096 bytes shorter, 1,195,459 bytes differ, carve-out at
`0x26ca000` instead of `0x26cb000`), and `9b6391b1` again. p2 is (8, 11, 17),
green 11, the reference.

`33a0f264..5ba988ee` adds the Mali skeleton (`src/mali/`,
`src/shaderc/online/mali/`, `tools/mali/`). Its changes to shared files
(`src/backend.rs`, `src/gl/{context,draw,egl,state}.rs`, `src/lib.rs`,
`src/shaderc/online/{backend,codegen}.rs`, `src/shaderc/precompiled.rs`) are
each behind `cfg(feature = "gpu-mali")` or are comments, except `lower_source`
in `codegen.rs`, which only becomes `pub(super)`. The cluster check
(`ef2fad0b`) is not in the Slint image. So nothing of it changes the V3D
Slint image. So the source of the SO image is the same at `33a0f264`
(`312b`) and at `5ba988ee` (`c592`), as it is at `c854e0ae` (`c592`) and
`499b946e` (`312b`). This is a second pair with no relevant change and a
different picture.

### What is left as the cause, and what was not measured

The picture is stable inside one session (SO and SV agree each time, and
three boots agree here) and changes between sessions. A commit cannot explain
that. What changes between sessions is outside the tree:

1. **Build inputs that are not in the tree.** A session builds on whichever
   farm machine it gets, with that machine's `SLINT_NOSTD_DIR`,
   `NOSTD_PATCH_DIR` (the femtovg and glow forks) and nightly `rustc`. The
   console's build line (`sample: build tree=... table=...`,
   `driver/zephyr/lib/board/common/sample_glue.c:53`) names only the yakogl
   tree. A different femtovg or Slint revision, or another compiler, changes
   the gradient arithmetic, and that moves a smooth dark gradient by one
   level at one probe, as seen. This machine builds with
   `rustc 1.98.0-nightly (4c9d2bfe4 2026-07-01)` and gives `c592`. The earlier
   sessions' machines and inputs are not in their records. **This is the
   first suspect.** It fits all five rows and is not measured.
2. **Board state.** The board reports undervoltage (`vcgencmd get_throttled`
   `0x50000` after each of the three runs here; the console prints
   `hwmon hwmon1: Undervoltage detected!`). It gave `c592` all three times
   anyway, so undervoltage alone does not give `312b`. Not excluded for other
   days.
3. **Image layout.** Two builds of one tree that differ in 1.2 MB and in
   their carve-out address gave the same picture, so a layout shift alone
   does not flip it.

Not done in this session: images of `c854e0ae`, `d4a8aee4`, `1eb9783d` and
`499b946e` were not built. This workspace could not check out another commit
(git checkout, worktree, archive and apply were refused), so only HEAD could
be built. The analysis above says those four images build the same code in
pairs. A board run of them would test the build inputs of this machine, not
the commits.

### What would settle it (proposals, no code changed here)

- Keep the SO image of the next run that gives `312b`, and boot that same
  image again on a later day. `312b` again: the image (its build inputs) is
  the cause. `c592`: the board is.
- Print the identity of the build inputs on the console next to the tree,
  for the Slint and cluster images: the Slint and femtovg revisions (or a
  hash of `SLINT_NOSTD_DIR` / `NOSTD_PATCH_DIR`) and `rustc -V`, for example
  in `common-images.sh`'s `.built` line and the sample's build line. Then a
  `312b` row can be matched to its inputs.
- Then build SO on two machines from one commit and compare the pictures.

## What to do

1. Keep the reference `c592` / `0xfaa46995` in the docs, RESULTS.md and
   `common-tree.txt`. Do not write `312b`.
2. The next Pi 4 common set (`pi4-final-board`) reads SO and SV: crc16
   `c592`, crc32 `0xfaa46995` is expected (`5ba988ee`, three runs). A
   `312b` there is the known session-to-session change of this note, not a
   regression of the commits since `5ba988ee`, unless those commits change
   `src/gl`, `src/v3d` or the Slint sample.
3. Do not bisect `312b` over commits again: the two windows with no relevant
   change (`c854e0ae..499b946e`, `33a0f264..5ba988ee`) both flip. Use the
   proposals above instead.
4. Stage-2 gap item 8 is closed by this note: the cross-board Slint picture
   is equal on all three boards on `c854e0ae` (`c592`, `0xfaa46995`). The
   gap note still lists item 8 open and the Pi 4 SO cell as `312b`; its next
   editor updates that (this item edits no other note).
