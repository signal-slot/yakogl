# i.MX8MP gc-case10 board session, first part (30 interleaved boots + 10 solo case 2)

Date: 2026-10-08. Item v3d-hh-48m. Board: verdin-imx8mp, `BOARD_BOOT=uboot-mmc`.
Integration commit: `0175bac1b4571c2b3360851f171d69c4571d49ee` (the run lines print `build fc46bc68`,
the item commit on top of it that adds only this doc).

Result: **case 10 failed in 1 of 30 runs (page arm, round 9); case 2 failed in 0 of 40 runs.** Base was
0/10, so the rate of "about 5 of 10" did not show in this session. Reading: row `0/10 | - | -`
("the rate fell in this session").

## Environment

`D=$CCFARM_DEPS_DIR` (here `/home/tasuku/.local/share/ccfarm/deps/yakogl`), the shared workspace and SDK
that zephyr-ws-setup.md built; nothing was built from scratch.

```sh
export ZEPHYR_WORKSPACE=$D/zephyr-ws ZEPHYR_VENV=$D/zephyr-ws/.venv \
       ZEPHYR_SDK_INSTALL_DIR=$D/zephyr-sdk-1.0.1 IMX8MP_OUT=$D/imx8mp-out
export CFLAGS=-O2 CXXFLAGS=-O2 BOARD_HOST=torizon@verdin-imx8mp
```

## Build commands (two deviations from the recipe, both needed)

```sh
LIST='-DCONFIG_GC_GL_TEST_SWEEP_LISTS="1-6,8-10\;10\;2"'
W=-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y
tools/cts/imx8mp-gltest-build.sh gc10-base   0 "$LIST" $W
tools/cts/imx8mp-gltest-build.sh gc10-page   0 "$LIST" $W -DCONFIG_YAKOGL_GC_ALLOC_PAGE_ALIGN=y
tools/cts/imx8mp-gltest-build.sh gc10-reuse0 0 "$LIST" $W -DCONFIG_YAKOGL_GC_POOL_NO_REUSE=y
```

Deviations (the recipe's commands fail as written; the recipe should be corrected by bench-recipes):

1. **`\;` instead of `;` in LIST.** With a bare `;` CMake treats the value as a list and drops the
   separators: the image got `CONFIG_GC_GL_TEST_SWEEP_LISTS="1-6,8-10102"`, list 0 became cases 8..47
   and SEL=512 ran a wide sweep. That attempt printed `record=594b07ff,00000000,ef7fff00,f9fcff7e` on
   `base-1` (not the recipe's shape) and was discarded. After the fix the built `.config` shows
   `"1-6,8-10;10;2"` (checked on all three images).
2. **`-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y`** on all three arms. Without it the image lacks
   `gc_wdog_arm: armed` and `tools/board.sh` refuses it (rc=3, "would not arm WDOG1"). tools/check.sh
   builds this image with the same option. It is the same for every arm, so the A/B is unaffected.
   The page and reuse0 flags were confirmed in each `.config`.

## Run commands

```sh
for i in 1 2 3 4 5 6 7 8 9 10; do
  for arm in base page reuse0; do
    SEL=512 BOARD_BOOT=uboot-mmc tools/board.sh imx8mp run \
      "$IMX8MP_OUT/gc10-$arm/zephyr/zephyr.bin" build/gc10 "$arm-$i"
  done
done
for i in 1 2 3 4 5 6 7 8 9 10; do   # optional part
  SEL=2 BOARD_BOOT=uboot-mmc tools/board.sh imx8mp run \
    "$IMX8MP_OUT/gc10-base/zephyr/zephyr.bin" build/gc10 "c2-$i"
done
```

Process notes: the loop was interrupted once when the worker restarted, after `base-2` finished and
while `page-2` was starting (no record from that start). It resumed with `page-2` (round 2 therefore ran
`page-2`, `reuse0-2` after `base-2` as normal order). Earlier discarded attempts (the wide-sweep
`base-1`, refusals for the missing watchdog marker, and a few runs aborted by me, whose stray one-shot
image I removed from `/var/yakogl`) produced no readings. Every run in the tables below has w1 =
`00000000` and w2 low 16 bits `ff00` (case runs: see below), so none had to be repeated.

## Every run

Verdict lines are `board.sh imx8mp <tag> rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled
result=record:SWEEP resets=0 record=<words> back_after=<n>s uptime=<n>s`; all runs had rc=0,
resets=0 and cycled. Representative full lines:

```
board.sh imx8mp base-1 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:SWEEP resets=0 record=594b07ff,00000000,0000ff00,0000077e back_after=47s uptime=28s
board.sh imx8mp page-9 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:SWEEP resets=0 record=594b07ff,00000000,0000ff00,0000037e back_after=47s uptime=29s
board.sh imx8mp c2-1 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,0002ff01,0000001d,00000000 back_after=33s uptime=23s
```

Record words per run (`594b07ff,00000000,0000ff00,` + w3):

Every line below is `board.sh imx8mp <tag> rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled
result=record:SWEEP resets=0 record=594b07ff,00000000,0000ff00,<w3> back_after=<a>s uptime=<u>s`
(from the verdict line each run stored in `build/gc10/<date>-<tag>-<hash>/INDEX.txt`):

| tag | w3 | back_after | uptime | case 10 | case 2 |
|---|---|---|---|---|---|
| base-1 | 0000077e | 47s | 28s | pass | pass |
| base-2 | 0000077e | 47s | 28s | pass | pass |
| base-3 | 0000077e | 46s | 27s | pass | pass |
| base-4 | 0000077e | 45s | 25s | pass | pass |
| base-5 | 0000077e | 47s | 27s | pass | pass |
| base-6 | 0000077e | 45s | 26s | pass | pass |
| base-7 | 0000077e | 47s | 27s | pass | pass |
| base-8 | 0000077e | 45s | 25s | pass | pass |
| base-9 | 0000077e | 46s | 27s | pass | pass |
| base-10 | 0000077e | 45s | 25s | pass | pass |
| page-1 | 0000077e | 39s | 22s | pass | pass |
| page-2 | 0000077e | 45s | 29s | pass | pass |
| page-3 | 0000077e | 38s | 21s | pass | pass |
| page-4 | 0000077e | 47s | 29s | pass | pass |
| page-5 | 0000077e | 47s | 31s | pass | pass |
| page-6 | 0000077e | 47s | 30s | pass | pass |
| page-7 | 0000077e | 37s | 21s | pass | pass |
| page-8 | 0000077e | 44s | 29s | pass | pass |
| page-9 | **0000037e** | 47s | 29s | **FAIL** | pass |
| page-10 | 0000077e | 45s | 29s | pass | pass |
| reuse0-1 | 0000077e | 45s | 28s | pass | pass |
| reuse0-2 | 0000077e | 37s | 21s | pass | pass |
| reuse0-3 | 0000077e | 45s | 29s | pass | pass |
| reuse0-4 | 0000077e | 45s | 28s | pass | pass |
| reuse0-5 | 0000077e | 45s | 29s | pass | pass |
| reuse0-6 | 0000077e | 38s | 21s | pass | pass |
| reuse0-7 | 0000077e | 47s | 30s | pass | pass |
| reuse0-8 | 0000077e | 47s | 31s | pass | pass |
| reuse0-9 | 0000077e | 45s | 29s | pass | pass |
| reuse0-10 | 0000077e | 47s | 29s | pass | pass |

Solo case 2 on base (`SEL=2`; CASE record, pass = `594b01xx,0002ff01,<checks>,00000000`). Every line is
`board.sh imx8mp c2-<i> rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:CASE
resets=absent record=594b010e,0002ff01,0000001d,00000000 back_after=<a>s uptime=<u>s` (pass, 0x1d = 29 checks made):

| tag | back_after | uptime |
|---|---|---|
| c2-1 | 33s | 23s |
| c2-2 | 34s | 22s |
| c2-3 | 39s | 27s |
| c2-4 | 34s | 22s |
| c2-5 | 39s | 27s |
| c2-6 | 34s | 22s |
| c2-7 | 33s | 23s |
| c2-8 | 34s | 22s |
| c2-9 | 39s | 28s |
| c2-10 | 34s | 22s |

## Counts

| arm | case 10 failed | case 2 failed |
|---|---|---|
| base | 0 / 10 | 0 / 10 (sweep) + 0 / 10 (solo) |
| page | 1 / 10 (round 9) | 0 / 10 |
| reuse0 | 0 / 10 | 0 / 10 |

## Reading

Row of the recipe table: **base 0/10 -> "the rate fell in this session"**. It does not match
the other rows, which assume base fails about 5 of 10 (14/30 before). The chance of 0/10 at a true rate of
0.47 is about 0.53^10 = 0.2 %, so either the failure rate depends on something that differed in this session
(the image now has WDOG self-arm; the board's state; the thermal or time-of-day state), or the earlier rate no longer holds. The page arm's single
failure shows the flake still exists (page alignment is not a cure); one failure is not a rate.
Nothing here supports (b) or (c): the arms are not distinguishable at these counts.

Next step named by the row: **run 20 more base boots before reading the others** (`SEL=512`, same
image, `base-11` to `base-30`). If base stays near 0/30 the flake needs a trigger this build/session
does not have (check the WDOG self-arm option as a candidate, since it is the one change against the
earlier measurement, by building the old image without it only if `board.sh` is told
`NETBOOT_NO_WATCHDOG=1`); if base returns to about 5/10, redo the table.

Not run: the second (K1) part, which waits for the knob.
