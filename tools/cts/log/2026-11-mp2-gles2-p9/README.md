<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# STM32MP257F-DK dEQP-GLES2 sweep (roadmap P9), six slices

The DK's dEQP-GLES2 sweep from `docs/notes/stm32mp257f-dk-p9.md` (dEQP section),
cut into six board sessions (slices). All six use the image built from
origin/main `66fcaa3a2c3ca8cc2aa0be19dbe43cd2e67ee35e`, and **one shard plan**.
Do not rebuild or re-plan between slices; only slice 2 stood in a rebuild at
the same commit, because slice 1's binary was not on its host (Image row below).

| | |
|---|---|
| Image | `tools/cts/board-stage.sh stm32mp257f-dk deqp` at `66fcaa3a`, i.e. `build/deqp/stm32mp257f-dk-66fcaa3a/zephyr/zephyr.bin`, 10,808,384 bytes, sha256 `b0fea5e819715d62509aca041d4fcbc4d75ef96962e813130e0fc4c56441e7cb`, footprint `0xa0000000..0xabee1000`. Slice 2 ran a rebuild at `66fcaa3a` on another host: the same 10,808,384 bytes and footprint, sha256 `9d9d78b4405f4311a61a138a9403e6b829808cf557816661daba3079e5bf1098`; builds at one commit are not byte-identical |
| Stage 1 | the DK's own (`driver/samples/zephyr/deqp/README.md`, DK section): VK-GL-CTS `93bca01861b0`, `BOARD=stm32mp257f_dk/stm32mp257fxx/a35 CFLAGS=-O2 CXXFLAGS=-O2 tools/cts/cts-build-zephyr.sh` |
| Case slot | `python3 -I tools/cts/mp2-deqp-slot.py show <bin>`: room 16,383 bytes (`CONFIG_DEQP_CASE_SLOT_SIZE=16384`). The largest shard list is 14,710 bytes (shard 15) |
| Lifetime | `CONFIG_YAKOGL_STM32MP2_LIFETIME_S=600` (600 s, 600,000 ms) |
| Runner bound | `RUN_BOUND=700` |
| Case list | `cases.txt`: `mp2-deqp-gles2.txt` (83 groups) minus `mp2-deqp-gles2-exclude.txt` (21 names), 17,144 cases. `tools/cts/mp2-deqp-list-check.sh` prints `ok, 83 groups, 21 excluded, 17144 mustpass cases to run`. The sweep's dry run prints `17144 cases` |
| Shard plan | `python3 -I tools/cts/mp2-deqp-shards.py cases.txt --out shards --budget-ms 12500` (no `--times`, so 100 ms a case): 138 shards, 137 with 125 cases and the last with 19. `manifest.tsv` maps shard to slice |

## Slices

The slices are 23 shards each, `manifest.tsv` column `slice`:

| Slice | Shards | Cases |
|---|---|---|
| 1 | 0..22 | 2,875 |
| 2 | 23..45 | 2,875 |
| 3 | 46..68 | 2,875 |
| 4 | 69..91 | 2,875 |
| 5 | 92..114 | 2,875 |
| 6 | 115..137 | 2,769 |

Each slice's shard logs go in `s<k>/shard-NNNN.log`, in the shape `mp2-deqp-sweep.sh`
writes: the runner's output followed by the run's `console.txt`.

`times.tsv` carries one row per run shard. Each later slice (s3..s6) adds its
own `sN/` directory the same way (`s3/shard-0046.log` .. `s6/shard-0137.log`)
and appends its shard rows to `times.tsv` with the same header and columns:
its shard indices are 46..137 and its slice number goes in the `slice` column.

Each file here without its own SPDX header has a `<file>.license` sidecar
(`tools/license_audit.py`, the gate's licence row), as the other log
directories under `tools/cts/log/` do. A later slice adds one per shard log.
Before the commit, run `python3 -I tools/release/redaction-scan.py HEAD`
(exit 0 or 3) on the slice: the ST kernel's boot log carries an e-mail address,
which is replaced with `<email>` (see step 4 below). The same commit
replaces the `mp2-p9-diff:` line of `docs/notes/stm32mp257f-dk-p9.md`
section 6 (the list-shard sweep) with the one over every landed slice:
`tools/cts/mp2-p9-notes-check.py` c3, which its self-test runs on the real
tree, fails otherwise.

## How a shard was run (slices 1-2, 2026-11-25 and 2026-11-26)

The A35 console is not a local device on the machine that drives the
runner. It is reached through the farm's serial tool. So each shard ran the
sweep's per-shard steps by hand:

1. `mp2-deqp-slot.py write <bin> --case-list shards/shard-NNNN.txt -o slot-NNNN.bin`.
2. `MP2_NO_CONSOLE=1 RUN_BOUND=700 tools/board-stm32mp2.sh run slot-NNNN.bin run/shard-NNNN shard-NNNN`
   (blind). Batches stopped at the first runner exit other than 0. None happened.
3. The serial capture of the run, from the Linux `reboot: Restarting system`
   before it to the next one, then went to
   `tools/board-stm32mp2.sh judge <rundir> <console>`. That call writes `console.txt` and
   gives the run's verdict line (`... row ... console=external`).
4. `shard-NNNN.log` holds the runner's output plus that `console.txt`. Before
   the commit, MAC addresses, the board's MAC-derived host name and workspace
   paths were replaced (`<mac>`, `<board-hostname>`, `<repo>`), and so was the
   e-mail address in the kernel's `pps_core` copyright line (`<email>`), which
   the redaction scan's e-mail class flags. The runner
   itself had already replaced IPv4 addresses and the ssh host.

The runner's U-Boot hook was installed before shard 0, with its undo
printed first, and uninstalled after shard 22; slice 2 installed it again
before shard 23 and uninstalled it after shard 45 (`install absent`, nothing
left behind). It was absent before each session, as it is now.

The serial tool's capture has no time stamps. `mp2-case-times.py` therefore
gives `cases 2875, timed 0, missing 0` for slice 1 and `cases 5750, timed 0,
missing 0` over slices 1-2, so there are no per-case times. The
per-shard time in `times.tsv` is the layer's own
`mp2: board: run over: PSCI SYSTEM_RESET at <n> ms`, which is the image's
uptime at the end of the run.

## Slice 1 results (shards 0..22, 2026-11-25)

Verdicts from the serial console, all 23 runs
`rc=0 verdict=done boot=uboot-sd row boot_verdict=complete`:

- **2,875 cases, 2,875 with a result: 2,855 Pass, 12 Fail, 8 NotSupported, 0 other, 0 without a result.**
- `mp2-p9-diff.py --report-only --exclude tools/cts/mp2-deqp-gles2-exclude.txt s1/shard-*.log`:
  `mp2-p9-diff: shared 0, new 12, fixed 0, missing 0, excluded 21`. The 26
  "accepted names that changed state" (`NotSupported -> NoResult`) are
  multisample, msrtt and compressedtexsubimage2d names: slice 1 had not run
  them yet, and slice 2 ran all 26 as `NotSupported`, as its section below shows.
- Fail (shard 10): `fbo.render.color.mix_tex2d_{rgb,rgba,bgra}` and
  `mix_npot_tex2d_{rgb,rgba,bgra}`, each also with `_depth_component16`, 12 in all.
  They draw a textured quad into the FBO, so they belong to the
  colour-texture class (b) of `docs/notes/stm32mp257f-dk-p9.md`. This is a Vivante
  back-end defect, listed here and not fixed.
- NotSupported (shard 0): the 8 `clip_control.*` cases. The i.MX8MP reports the same.
- No hang, no refusal, and no run cut by the lifetime.

Times against the 600 s lifetime (`times.tsv`, shards 0..22): 22 of the 23 shards took 4.1 to
19.6 s. Shard 13 took 159.5 s, 26.6 % of the lifetime. It holds the five
`flush_finish.*` cases, which wait by design, followed by 106
`fragment_ops.blend` cases. The sum is 334.0 s of image time. Each run took
66 to 227 s of wall time, reboot and return to ssh included (`back_after`
47 to 207 s). The 12,500 ms budget therefore leaves at least 73 % of the lifetime
free in every slice-1 shard.

## Slice 2 results (shards 23..45, 2026-11-26)

Verdicts from the serial console, all 23 runs
`rc=0 verdict=done boot=uboot-sd row boot_verdict=complete`; each log's
`deqp: summary` line reads `125 cases begun, 125 with a result, 0 without`:

- **2,875 cases, 2,875 with a result: 2,741 Pass, 105 Fail, 26 NotSupported, 3 QualityWarning.**
- Fail (13 shards: 26, 27, 31, 32, 33, 34, 35, 37, 40, 41, 42, 43, 45), all
  new against the i.MX8MP and almost all `*_fragment` cases: the
  fragment-arithmetic class (a) of `docs/notes/stm32mp257f-dk-p9.md` (the f20
  immediate lane-z misread) — `fragment_ops.interaction.basic_shader` (14,
  shards 26 and 27), `shaders.constant_expressions.builtin_functions`
  exponential/geometric/matrix/trigonometry (39) and complex_types (1),
  `shaders.indexing.matrix_subscript` (19) and tmp_array (8),
  `shaders.functions` array_arguments/datatypes/misc (13),
  `shaders.conversions` vector_to_vector/matrix_combine (9),
  `shaders.algorithm.hsl_to_rgb_fragment` and
  `shaders.builtin_variable.fragcoord_xyz` (1 each). These are Vivante
  back-end defects, listed here and not fixed.
- NotSupported (shards 29 and 30): the 16 `multisample.*`, the 1
  `multisampled_render_to_texture.readpixels` and the 9
  `negative_api.texture.compressedtexsubimage2d_*` names, 26 in all — exactly
  the names listed as `accepted` state changes under slice 1. The i.MX8MP
  reports the same 26 `NotSupported`.
- QualityWarning (shard 31): `rasterization.interpolation.basic.line_strip`,
  `rasterization.interpolation.basic.lines` and
  `rasterization.interpolation.projected.lines`, 3 in all. The i.MX8MP fails
  these three; they are the diff's 3 `fixed`.
- No hang, no refusal, and no run cut by the lifetime.

Times against the 600 s lifetime (`times.tsv`, shards 23..45): all 23 shards
took 3.5 to 17.6 s, at most 2.9 % of the lifetime; the sum is 193.4 s of
image time. Each run took 80 to 108 s of wall time, reboot and return to ssh
included (`back_after` 45 to 73 s).

## Slices 1-2 together (shards 0..45)

- `deqp-uart-tally.py --mustpass cases.txt s1/shard-*.log s2/shard-*.log`:
  5,750 mustpass cases with a result, **5,596 Pass, 117 Fail, 34
  NotSupported, 3 QualityWarning**. `Fail 117` = slice 1's 12 + slice 2's
  105, all of them DK-only (`shared 0`); `NotSupported 34` = the 8
  `clip_control.*` + the 26 above.
- `mp2-p9-diff.py --report-only --exclude tools/cts/mp2-deqp-gles2-exclude.txt s1/shard-*.log s2/shard-*.log`:
  `mp2-p9-diff: shared 0, new 117, fixed 3, missing 0, excluded 21`, with 0
  accepted names that changed state. This is the line quoted in section 6 of
  `docs/notes/stm32mp257f-dk-p9.md`; `mp2-p9-notes-check.py` c3 checks it, as
  said above.
- Times against the 600 s lifetime (`times.tsv`, 46 rows): 45 of the 46
  shards took 3.5 to 19.6 s, at most 3.3 % of the lifetime; shard 13 remains
  the exception at 159.5 s, 26.6 %. The 46 runs sum to 527.3 s of image time.
  Each run's `back_after` is 45 to 207 s. The 12,500 ms budget therefore
  leaves at least 73 % of the lifetime free in every shard run so far.

## Slice 6 (shards 115..137, 2026-11-28)

Image: a rebuild of origin/main `66fcaa3a` on another host (stage 1 VK-GL-CTS
`93bca01861b0`, `-O2`; stage 2 `board-stage.sh stm32mp257f-dk deqp`): the same
10,808,384 bytes and footprint `0xa0000000..0xabee1000`, sha256
`7c5cd67f5453b1d7af9622684aecb3193b98f2932b64d498f7cea53664592e28`. Shard plan,
lifetime and `RUN_BOUND` as above; the hook was installed before shard 115 and
uninstalled after shard 137 (`install absent`).

The serial tool returns each shard's capture whole (saved to a file when it is
large), and the logs are that capture: the runner's output, then `console.txt`
from `judge` over the capture from the Linux `reboot` line to the end of the
read, with the redactions of step 4 and with non-ASCII bytes and CRs dropped.
Every `deqp:` line is as the console printed it; nothing is written from totals.
In shard 132 the serial line lost the `reboot: Restarting system` line and a few
bytes after it; the capture there starts at `systemd-shutdown: Rebooting.`.

- **2,769 cases, 2,769 with a result: 2,292 Pass, 447 Fail, 0 NotSupported, 30 QualityWarning.**
  All Fail and QualityWarning are `functional.texture.*`.
- `mp2-p9-diff.py --report-only --exclude tools/cts/mp2-deqp-gles2-exclude.txt s*/shard-*.log`:
  `mp2-p9-diff: shared 12, new 1348, fixed 5, missing 0, excluded 21`, 0 accepted
  names that changed state, 0 mustpass names missing: all 17,144 cases have a result
  (15,717 Pass, 1,360 Fail, 34 NotSupported, 33 QualityWarning).
- Times: image time 2.7 to 45.1 s except shard 117 at 413.3 s (68.9 % of the
  lifetime, back_after 474 s). `times.tsv` has the 23 rows.
