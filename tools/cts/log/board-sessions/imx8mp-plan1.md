<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP board session: plan #1 on the integration image

**Integration image, not a landing proof.** Source: the integration
snapshot, HEAD `b20e4d11e595c2734671132aac34c468b0dfbaa4` (`b20e4d11`).
`4e9397c680b4cf2ebf09425c3357c705e52478c4` (fbo, fragdata),
`6bddfc5380177da2b8f79297646dbaf00a030a57` (case 16, the bit 3 A/B) and
`84094196f46485f6225b30b13a4659ef65bd7aeb` (#9, #25) are all ancestors of HEAD
(checked with `git merge-base --is-ancestor`). A later origin/main session
repeats this before landing.

Date: 2026-10-08, worker host B. Board: Verdin i.MX8M Plus,
leased resource `<board>`, `BOARD_BOOT=uboot-mmc`. Steps 1, 2, 3 and 5
of `docs/HANDOFF.md` "Next board session → i.MX8MP" all ran; step 4 (the
whole tree) was not part of this item. Every image was built in this
workspace from `b20e4d11` plus a docs-only commit (no build input differs);
`board.sh` records `build d5b578ba` on every run — the hash this document's
commit had while the runs were made, before the results were amended into it.
Records: `records/imx8mp-plan1/`.

## Summary

| step | what | verdict |
|---|---|---|
| 1 | dEQP mini-table: shaders tail + fbo | both rows at the `29460f96` numbers: 1512/1512/0 and 711/711/0 — `4e9397c` confirmed |
| 2 | gl_test default image, 3 runs | 46/48 all three runs, first failure case 16 — the record is byte-identical to the baseline; a supplementary single-case run shows case 16 is now a skip, not a pixel failure |
| 3 | 4x MSAA A/B, case 16 alone, arms 1 and 9 | both arms fail at check 4 `x4 inside` with got 0x00000000: bit 3 does not fix case 16; arms 11/13 not run (only run if 9 passes) |
| 5 | #9/#25 repeat runs, 60 boots | base 4/40 (3× case 10 #9, 1× case 2 #25), page 1/10 (#9), reuse0 2/10 (#9, #25) — all arms still fail, at a far lower rate than the earlier 14/30 |

## Step 1: dEQP mini-table — both rows back at the 29460f96 numbers

Stage 1 was built on this node from VK-GL-CTS `3c6d81d81d537338be16fa99cb6af59dc8d0b338`,
a codeload-tarball import of the pinned CTS `93bca01861b0e3ef3c387027a9791e6d065f900c`
("Merge vk-gl-cts/vulkan-cts-1.4.6 into vk-gl-cts/main", 2026-09-18). After the
runs the pinned commit was fetched (`git fetch --depth 1`) and compared with the
import: the trees differ only in three files the build does not use
(`.clang-format`, `.gitattributes`, `doc/testspecs/VK/apitests-docinfo.html`);
every file under `external/`, `framework/` and `modules/` is byte-identical, so
stage 1 was built at the pinned revision. Both rows' case counts (1512, 711)
also match `tools/cts/imx8mp-full-tree-29460f96.results.tsv` exactly.

Image `fbo-b20e4d11` (`imx8mp-deqp-build.sh fbo-b20e4d11 'dEQP-GLES2.info.*'
--table <mini-table>`, the mini-table being the 3rd and 22nd non-comment lines
of `tools/cts/imx8mp-full-tree.txt`): conformance check passed
(`check-deqp-images: the dEQP image recipes build what conformance needs`),
`zephyr.bin` sha256 `617a33bc…`, footprint 221282304.

```
board.sh imx8mp e00 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0890,000005e8,00000000,ffff05e8 back_after=46s uptime=21s
board.sh imx8mp e01 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b084e,000002c7,00000000,ffff02c7 back_after=59s uptime=29s
TALLY (2 entries): begun 2223 passed 2223 failed 0 notsup 0 other 0 noresult 0
```

- e00 (shaders tail, holds `shaders.fragdata.*`): begun 1512, passed 1512,
  failed 0, NotSupported 0.
- e01 (`dEQP-GLES2.functional.fbo.*`): begun 711, passed 711, failed 0,
  NotSupported 0.

At `72d16864` these rows were 1512 begun / 1511 passed / 1 NotSupported
(`shaders.fragdata.*`) and 711 / 710 / 1 fail
(`fbo.completeness.renderable.renderbuffer.depth.depth_component24`). Both are
back at the `29460f96` numbers with no failing names: **`4e9397c` is confirmed
on the board.**

## Step 2: gl_test default image, three runs

GT image from `tools/cts/common-images.sh` (default `prj.conf`, watchdog
self-arm), sha256 `cff6d9d5…`, build stamp 2026-10-08T02:54:45Z. All three
runs:

```
board.sh imx8mp gt-b20e4d11-1 rc=4 verdict=failed boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,0010ff06,0000002e,00000030 back_after=64s uptime=27s
board.sh imx8mp gt-b20e4d11-2 rc=4 verdict=failed boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,0010ff06,0000002e,00000030 back_after=59s uptime=21s
board.sh imx8mp gt-b20e4d11-3 rc=4 verdict=failed boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,0010ff06,0000002e,00000030 back_after=64s uptime=26s
```

Decoded: `CASE case 16 check 255 status 6 (set failed) got 0x2e (46) want
0x30 (48)` — 46 of 48 cases passed, the set's first failure is case 16. The
record is byte-identical to the baseline2-imx8mp record
(`tools/cts/log/benchmarks-baseline/imx8mp.md`).

- Cases 2 and 10 (#25, #9) passed in all three runs: they run before case 16
  in the set's order, and the first failure is 16 in every run. No flip in
  these three runs.
- A second case failed in each run (46 of 48) and is not named by the record
  (it runs after case 16; the SET_FAIL record names only the first failure).

**Case 16 reading.** The set record cannot show the `6bddfc5` fix: the set
walker counts any non-zero case return as a failure —
`if (crc != 0) { fails++; … }` in `driver/tools/pvrlab/gl_test.c` — and the
by-name skip returns -3, so case 16 stays the record's "first failure" even
though it no longer fails a check. A supplementary run proves what case 16
actually does now: the `msaaoff` image (no `CONFIG_GC_GL_TEST_MSAA`, so no 4x
EGL config, the same case-16 code path as the default image) with `SEL=16`
(case 16 alone) recorded

```
CASE (1) seq 14: case 16 check 255 status 3 (failed, no check) got 0x00000006 want 0x00000000
```

— its 1x half ran and passed all six checks, then the 4x half was skipped by
name (-3). Case 16 is a skip in the default image, not a pixel failure; only
the walker's counting keeps it in the set's failure tally.

## Step 3: 4x MSAA A/B, case 16 alone (`SEL=16`)

Images `msaa1-b20e4d11` (`-DCONFIG_GC_GL_TEST_MSAA=y
-DCONFIG_GC_GL_TEST_MSAA_BITS=1`) and `msaa9-b20e4d11` (bits 9 = bit 0 + bit
3), plus the `msaaoff-b20e4d11` control above.

**Recipe finding first:** `tools/cts/imx8mp-gltest-build.sh` does not enable
`CONFIG_GC_BOARD_WDOG_SELF_ARM`, so its images lack the `gc_wdog_arm: armed`
marker and `tools/board.sh` refuses them (`rc=3`, "it would not arm WDOG1").
The first msaa1 run was refused; all three images were rebuilt with
`-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y` added. HANDOFF's step 3 and step 5 recipes
need that flag.

```
board.sh imx8mp msaa1 rc=4 verdict=failed boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,00100402,00000000,ffff00ff back_after=33s uptime=25s
board.sh imx8mp msaa9 rc=4 verdict=failed boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,00100402,00000000,ffff00ff back_after=34s uptime=24s
```

Decoded, both arms identically: `CASE case 16 check 4 status 2 (check failed)
got 0x00000000 want 0xffff00ff`. Check 4 is `x4 inside` (the case's checks in
order: 1 `eglChooseConfig samples 4`, 2 `config EGL_SAMPLES`, 3 `GL_SAMPLES
x4`, 4 `x4 inside`).

Reading against HANDOFF's table:

- Arm 1 (bit 3 off) expected `x4 inside` 0xff000000 — the clear colour, a
  quarter-size picture in the corner. Actual: 0x00000000. The resolved target
  is blank (alpha 0 too): neither the clear nor the triangle landed — not even
  the quarter-size clear signature the table expects for this arm.
- Arm 9 (bit 0 + bit 3, viewport/clip scaled to the 2x scratch) expected case
  16 to pass. Actual: the identical failure. **Bit 3 does not fix case 16.**
  Per HANDOFF: bit 3 stays off and #8 stays open. Arms 11 and 13 were not run
  (the recipe runs them only if arm 9 passes).
- Limitation: the `x4 scratch`, `x4 clear only` and `x4 draw only` map lines
  that HANDOFF's table uses to tell a failed resolve from a failed draw are
  `pr_info` output, and this board has no console (records only). The
  localization between "blank target with a full scratch (the resolve)" and
  "empty scratch (the draw)" could not be made from this session. What the
  record does say: with the 4x path on, the target reads blank in both arms.

## Step 5: #9/#25 repeat runs (`SEL=512`, list 1-6,8-10)

Images `gc9-base`, `gc9-page` (`-DCONFIG_YAKOGL_GC_ALLOC_PAGE_ALIGN=y`),
`gc9-reuse0` (`-DCONFIG_YAKOGL_GC_POOL_NO_REUSE=y`), each
`-DCONFIG_GC_GL_TEST_SWEEP_LISTS="1-6,8-10" -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y`
(the Kconfig string needs its quotes kept through cmake; the self-arm flag as
in step 3). Each run ends with the counters record; all 60 runs below have
w1 = 0 (lost work 0, GPU resets 0, fence timeouts 0), so every one is a
reading. Pass bits: `0x77e` all nine cases passed, `0x37e` case 10 failed
(#9), `0x77a` case 2 failed (#25).

First 30 boots (10 per arm):

| arm | failures | which |
|---|---|---|
| base | 0/10 | — |
| page | 1/10 | page-3: case 10 (`0x37e`) |
| reuse0 | 2/10 | reuse0-4: case 10 (`0x37e`); reuse0-6: case 2 (`0x77a`) |

Example verdict line (all 60 have this shape):

```
board.sh imx8mp base-3 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:SWEEP resets=0 record=594b07ff,00000000,0000ff00,0000077e back_after=46s uptime=29s
```

HANDOFF expected base to fail case 10 in about 5 of 10 runs (14 of 30
earlier). Base was 0/10, so the table's contingency ran: 30 more boots on the
same base image (`base-11` … `base-40`).

Extra 30 base boots (`base-11` … `base-40`, the contingency): 4 failures in
30 — `base-13` case 2 (`0x77a`, #25), `base-20`, `base-23`, `base-35` case 10
(`0x37e`, #9). Base total: **4/40** (3× #9, 1× #25).

Reading: the base rate is 4/40 (10 %), not the expected about 5/10 (14/30 ≈
47 % earlier): it fell a lot but did not collapse — base still fails. At this
sample size the arms do not separate: page 1/10, reuse0 2/10 and base 4/40 are
all low and within noise of each other, and neither knob removed the failure
(page alignment kept #9 at 1/10, pool no-reuse kept it at 1/10). The closest
outcome row is "neither; (a) is left" — but with rates far below the about
5/10 that row assumes. The drop itself is the finding: something between the
14/30 measurement and `b20e4d11` lowered the default path's case-10 rate, and
the (b)/(c) discrimination needs a re-measure (or a bisect) against the commit
the 14/30 was taken at before it can be read from this table.

## Findings for the follow-up

1. `tools/cts/imx8mp-gltest-build.sh` images need
   `-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y` before `tools/board.sh` will run them;
   HANDOFF's step 3 and step 5 recipes omit it. (The `SWEEP_LISTS` Kconfig
   string also needs its quotes preserved through cmake.)
2. The set walker counts the by-name skip (-3) as a failure, so the SET_FAIL
   record cannot show the `6bddfc5` harness fix (step 2). Counting a -3 like a
   config skip (the `case_skipped` path already does) would make the record
   say what the case now does.
3. #8 (4x MSAA): bit 3 does not fix case 16; both arms leave the resolved
   target blank (0x00000000). Telling resolve from draw needs the scratch
   probes, which need a console — or a record kind for them — on the i.MX8MP.
4. #9/#25: on `b20e4d11` the base arm fails in 4 of 40 boots (3× case 10, 1×
   case 2; was 14 of 30), and #25 also appears in reuse0 (1 of 10). Neither
   knob (page align, no reuse) removes the failures; at these rates the step 5
   table cannot discriminate (b) from (c) until the rate is re-measured
   against the commit the 14/30 was taken at.

## How to repeat

From a detached worktree of `b20e4d11` (no fetch), with the i.MX8MP build
variables of `tools/cts/imx8mp-env.sh` exported (`ZEPHYR_WORKSPACE`,
`ZEPHYR_VENV`, `ZEPHYR_SDK_INSTALL_DIR`, `IMX8MP_OUT`) plus `DEQP_STAGE1`,
`PATH` carrying a real
cargo (a cargo shim without a rustc beside it breaks the capi build),
`RUSTUP_TOOLCHAIN=nightly` on build commands only, and `BOARD_HOST` /
`BOARD_BOOT=uboot-mmc` from the leased resource:

```sh
# step 1 (stage 1 first if none exists for the CTS revision in use:
# tools/cts/imx8mp-deqp-stage1.sh, which reads ZEPHYR_SDK and wants CFLAGS=-O2)
grep -vE '^[[:space:]]*(#|$)' tools/cts/imx8mp-full-tree.txt | sed -n '3p;22p' > $IMX8MP_OUT/mini-table.txt
tools/cts/imx8mp-deqp-build.sh fbo-$HEAD8 'dEQP-GLES2.info.*' --table $IMX8MP_OUT/mini-table.txt
TREE_IMAGE=$IMX8MP_OUT/fbo-$HEAD8/zephyr/zephyr.bin tools/board.sh imx8mp tree $IMX8MP_OUT/mini-table.txt <rec>/dEQP
# step 2
tools/cts/common-images.sh imx8mp tools/cts/common-tree.txt <img> GT
tools/board.sh imx8mp run <img>/GT/zephyr/zephyr.bin <rec>/gt gt-$HEAD8-$i   # x3
# step 3 (add the self-arm flag; SEL=16)
tools/cts/imx8mp-gltest-build.sh msaa$bits-$HEAD8 0 \
  -DCONFIG_GC_GL_TEST_MSAA=y -DCONFIG_GC_GL_TEST_MSAA_BITS=$bits -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y
SEL=16 tools/board.sh imx8mp run $IMX8MP_OUT/msaa$bits-$HEAD8/zephyr/zephyr.bin <rec>/msaa msaa$bits
# step 5 (quotes on the list; self-arm flag; SEL=512)
tools/cts/imx8mp-gltest-build.sh gc9-$arm 0 -DCONFIG_GC_GL_TEST_SWEEP_LISTS="1-6,8-10" -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y [arm flag]
SEL=512 tools/board.sh imx8mp run $IMX8MP_OUT/gc9-$arm/zephyr/zephyr.bin <rec>/gc9 $arm-$i
```
