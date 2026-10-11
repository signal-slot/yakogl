<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gc_gl_test -- the GL suite on the i.MX8MP's Vivante GC7000UL

Status: finished record, moved here from `driver/samples/zephyr/gc_gl_test/README.md` on 2026-10-11. The gc_gl_test diary of 2026-09-19 to 10-04 on the crash/kexec route: hold-time readings, results per case, host-build notes. Runs now end in the SNVS result record and boot through U-Boot; the sample's README says how to build and run it, and current results are tools/cts/STATUS.md.

`tools/pvrlab/gl_test.c` is the on-silicon suite the PowerVR side is held to:
forty-odd cases, every one checked by reading pixels back, covering uniforms,
textures, depth and stencil, cull, scissor, FBOs, blending and colour masks,
every primitive mode, client arrays, VAOs, EGLImage and the stencil-then-cover
fills FemtoVG draws with. **The same file compiles here.** It is GL-level and
portable; only the few lines that start the backend know which GPU this is,
and they are behind `CONFIG_YAKOGL_GPU_VIVANTE`.

Running it is the point. A triangle proves the pipe; this proves the backend,
case by case, so a failure names itself instead of arriving as "the UI is
black".

```sh
build/run_gc_gl_test.sh                                  # the regression set
build/run_gc_gl_test.sh -DCONFIG_GC_GL_TEST_CASE=9    # one case
```

## Build it without the display, always

The M7 owns the panel now, so a gl_test image must not carry the display
driver. Every build takes both of these:

```
-DCONFIG_SAFEUI_DISPLAY_IMX8MP=n -DCONFIG_GC_COMMON_PANEL_CONSOLE=n
```

and the old `WAIT_DISPLAY` gate in the runners is not to be used any more.

To check an image really is free of it, look for the driver's **functions**:

```sh
nm build/<dir>/zephyr/zephyr.elf | grep -c 'imx8mp_display_'    # must be 0
```

Do not grep for `imx8mp_display` loosely. That matches
`CONFIG_DT_HAS_SAFEUI_IMX8MP_DISPLAY_ENABLED`, which `nm` reports with type
`A` -- an absolute Kconfig symbol, not code. It is set because the devicetree
has a node with that compatible, which stays true whether or not the driver is
built, so the loose grep returns 1 on a perfectly clean image and reads like a
failure.

## The test knobs

This image selects `CONFIG_YAKOGL_TEST_KNOBS` (`GC_GL_TEST_KNOBS`, Kconfig),
the library's `test-knobs` feature: the GC7000 switches and probes the suite
sets to run an experiment's arms. Today `gc_debug_set_msaa` and the 4x
scratch probes (case 16, `CONFIG_GC_GL_TEST_MSAA`),
`gc_debug_set_narrow_la` (case 132's RGBA8 arm; the narrow store is the
default) and
`gc_debug_set_flip_experiment` (case 88 rsflip). gl_test.c calls
them only under that define, and a product image has none of them
(tools/check.sh, "no test knob in the artefact").

The A/B options for behaviour that is now the design are gone with their
switches (2026-10-06): `GC_GL_TEST_MASKED_CLEAR`, `GC_GL_TEST_VS_UNITS16`,
`GC_GL_TEST_NO_FIRST_COMPONENT_USE`, `GC_GL_TEST_NO_BLACK_UNBOUND`,
`GC_GL_TEST_MRT` (every colour attachment is written; case 106 no longer
skips) and `GC_GL_TEST_DESC_SEQ` (Mesa's descriptor order, its default). A
run script that still passes one of them stops at Kconfig.

## Reading a run

**Since 2026-10-04 the reading is the result record**, the four words in the
SNVS that survive the SoC reset (`yakogl-tools record`): the case and its first
failing check with what it read and wanted, or the set's summary (status
CASE_SET: passed of run, the first two failing cases, how many cases skipped
by name and the first of them; a skip is not run and not a failure, so a set
passes when passed == run), or UNFINISHED with the case a stalled suite was in.
The image resets the SoC right after writing it (`gc_sample_finish_recorded`)
with `CONFIG_GC_GL_TEST_SETTLE_SECONDS` at its default 0, so a one-case run
costs its boot and the case. Before that, a run held for a settle of 100 s and
then a coded hold, and the regression set came back after about 400 s.

The rest of this section is how runs were read before the record, and how the
diagnostic knobs whose answer has no record field (`REPORT_COUNTERS`,
`DEPTH_VALUE`, `DEPTH_PROBE`, and `REPORT_CHECK` / `REPORT_CHECK_COLOUR`, whose
answer the record now also carries) are still read: by the time the board
takes to come back. Set `SETTLE_SECONDS=100` with them. A coded hold is
`70 + 6 * code` seconds since 2026-09-21; readings quoted from before that
date used a 3 s step and mean twice the code they would mean now. The rows for
the plain run, the regression set and an unfinished suite describe images
before 2026-10-04.

| run | reading |
| --- | --- |
| one case | the plain holds: PASS is about 66 s, FAIL about 46 s |
| one case, `GC_GL_TEST_REPORT_CHECK=y` | 82 + 6 * (the first failing check), and 82 s when the case passed (code 1) |
| the same, `GC_GL_TEST_REPORT_CHECK_COLOUR=y` | 70 + 6 * (32 + the colour that check read): 262 s black, 268 blue, 274 green, 286 red, 304 white |
| the regression set | `SETTLE` (100 s) + 70 + 6 * the number of cases that PASSED |
| the regression set, `GC_GL_TEST_REPORT_FIRST_FAIL=y` (removed 2026-10-04: the record names the first failure) | the same, with the code being the first failing case's number |
| the suite did not finish | `DEADLINE` (200 s) + 70 + 6 * the case it was in |
| case 53 with `GC_GL_TEST_DEPTH_VALUE=y` | 70 + 6 * (the stored depth word >> 26). Each expected value spans **two** codes, because one near the top of a code's range rounds either way: **31 or 32** is the 0.5 the case clears to, 15 or 16 a quarter, 0 the near end, 63 a buffer still holding 1.0. 41 is the guard's canary, meaning the GPU wrote none of the surface. Needs `YAKOGL_VIVANTE_DEPTH_GUARD=y`, which is what maps the surface |
| any run, `GC_GL_TEST_REPORT_COUNTERS=y` | 70 + 6 * (bit 5: a submit found all 30 events in flight; bits 0-4: fence timeouts, saturating at 31) |
| about 9-11 s | the jump never happened; run it again -- about one run in three on the plain kexec route, and none in 18 on the crash route below |
| about 132 s | WDOG1 fired (124-128 s) plus the boot: the image hung |
| more than about 10 minutes | **the board did not come back.** Not a reading at all: longer than any hold the sample can ask for and longer than WDOG1, so nothing in the image can produce it. Rerun, and see below |

**The whole formula, and the constant that is easy to get wrong.** Every coded
return is

```
total = 4 + max(SETTLE, how long the suite itself took) + 70 + 6 * code
```

with the reading rounded to the nearest whole code: totals sit within about
2 s of the formula, and the code step is 6, so a rounded reading is never
ambiguous.

The 4 is the board's own boot and reset either side of the hold. Two values
were in use here, 4 and 6; 4 is the one the board gives, measured across runs
whose code was known independently:

| code | returns measured | 4 + 100 + 70 + 6*code | 6 + 100 + 70 + 6*code |
| --- | --- | --- | --- |
| 1 | 179, 179, 180, 180 | **180** | 182 |
| 2 | 186 | **186** | 188 |
| 6 | 210 | **210** | 212 |
| 32 | 366 | **366** | 368 |
| 33 | 372 | **372** | 374 |

`SETTLE` is `CONFIG_GC_GL_TEST_SETTLE_SECONDS`, 0 in the older rows here and
100 in the newer ones, and it is a floor rather than a term: the suite runs
first and the settle only waits out whatever is left. **That is why `SETTLE`
is not optional once a case's duration can vary** -- with `SETTLE=0` the
case's own seconds land in the total and the reading moves by a code step or
two. A case that hangs kicks can take 8 s or 19 s; the step is 6.

`70 + 6 * code` is `gc_sample_hold_code()` itself (`gc_sample.c:92`, with
`HOLD_CODE_STEP` 6).

Worked example: case 4 returned `kernel started 210 s after the jump` with
`SETTLE=100`. `(210 - 174) / 6 = 6.0`, so code 6, and with `REPORT_CHECK=y`
code 6 means check 5 failed.

With `REPORT_CHECK=y` the code is 1 when the case passed and 1 + n when check n
failed, so a pass is `80 + SETTLE` and check n is `80 + SETTLE + 6 * n`.

And decode against the sample's *own* `.config`, not the usual default:
`gc_slint` sets `GC_SAMPLE_HOLD_SECONDS=5`, so its pass returns at 64 s where
the common 60 would predict 119.

**A case is one bit unless you ask for more.** PASS or FAIL is enough to see a
fix land and not enough to find one: case 2 alone makes twenty-seven checks --
four texture sizes, a gradient whose pixel values name the texel fetched,
three mip levels, two texture units -- and they fail for different reasons.
`GC_GL_TEST_REPORT_CHECK=y` puts the first failing check in the hold instead,
numbered in the order the checks run (so inserting one renumbers the rest):
code 1 means the case passed, code 1 + n that check n failed, and code 63 that
it failed with no check saying so -- a GL error, a shader that would not link,
or a check numbered past 61. A board run costs minutes on a shared board; six
bits out of it instead of one is worth the switch.

Once the number is known it stops being news and what the check *read* starts
being: a mip level that comes back green where blue was wanted says the LOD
stopped a level short, and black says nothing was drawn at all. Add
`GC_GL_TEST_REPORT_CHECK_COLOUR=y` and the hold carries that instead, as 32
plus one bit per channel at half scale.

**A stall and a finished run are easy to confuse, and the confusion costs a
wrong conclusion.** Both end in a coded hold, and with `SETTLE` at 100 and
`DEADLINE` at 200 their totals overlap: a stall at case 9 and a finished run
with 25 passes both return about 328 s. Worse, the deadline path reports the
case it was in *whether or not* `REPORT_FIRST_FAIL` is set, so a pass-count
run and a first-fail run that both stalled return the same number and look
like agreement.

Tell them apart by moving the deadline: run again with
`-DCONFIG_GC_GL_TEST_DEADLINE_SECONDS=400`. If the total moves by about 200 s
the run was stalling; if it does not move, the suite finished and the code is
what it says. Measured 2026-09-21: 328 s at `DEADLINE=200` became 539 s at
`DEADLINE=400`, so those runs were stalls, not a pass count.

With a diagnostic knob, a finished run waits out `SETTLE` before its verdict so
that the suite's own duration is not a second unknown in the total.

The suite takes longer than the watchdog Linux leaves armed (about 128 s), so
it cannot be fed from the work: a thread of its own feeds it and gives up at
`DEADLINE`.

## When the board does not come back

Twice on 2026-09-21 a run left the board unreachable for about half an hour
before it rebooted -- once on case 8, once on case 15, both on the crash
route. That is a third outcome beside PASS and FAIL and it has to be told
apart from them: no hold the sample can ask for reaches ten minutes, and the
watchdog fires at 128 s, so nothing inside the image can produce a gap that
long. It is the board, not the run.

It also wedges whatever is waiting for it. The kexec call reads the loader's
output looking for "jumping", and the board goes away in the middle of that by
design; if the connection is left half open rather than closed, that read
blocks for as long as the board is gone -- with the board lock held, since the
lock is taken before the build. Every other cap in the runner is measured from
the jump, and the jump has not happened yet, so none of them applies. Bound
that call itself (`timeout "${JUMP_WAIT:-120}"`).

The runner now prints a timeline for every run, because "it did not come back"
says nothing about why:

    timeline: jump 0s, ping lost 3s, ping back 41s, ssh back 48s, route --crash

ping first: the board answers ICMP before sshd is up, so a long gap to *ping
back* is the board still down, while ping returning early with ssh never
following is the board up and not serving. Two different faults.

What is ruled out so far: the Ethernet DMA that had to be quiesced on the
AM62P is quiesced on **both** routes here -- `crash_jump()` calls
`quiesce_bus_masters()` before its `sysrq-trigger c`, exactly as the normal
path calls it before `reboot(KEXEC)`. What the crash route still skips is
`device_shutdown()` for everything else: USB, SD/eMMC, the GPU, the LCDIF.

One reboot in the timeline will not be a run: installing route 3 reboots the
board itself.

## The crash route, and the 64 MiB it needs

The 9-11 s no-jump return is Linux panicking inside the normal kexec path
(`kernel.panic=5`, plus the boot). The loader's `--crash` takes the kernel's
crash route through sysrq-c instead, which skips `device_shutdown()`; the
display driver adopts a running chain either way. Measured 2026-09-21 on
`run_gc_gl_test.sh`: **6 flakes in 14 runs before, none in 18 after.** It is
that runner's default now (`KEXEC_CRASH=0` goes back).

**It only works for an image that fits inside the `crashkernel=` reservation**,
which is `64M@0x60000000` on this board. `gc_gl_test` is about 32 MiB and
fits. `gc_slint` is about 99 MiB and does not: the loader says "sysrq crash did
not jump (crashkernel= reserved? image inside it?)" and the runner reports
"no jump". So `run_gc_slint.sh` leaves the crash route off and keeps the flake.

## The base, measured, for `SETTLE=100`

Every reading below this line was taken with
`CONFIG_GC_GL_TEST_SETTLE_SECONDS=100`, which is the base worth pinning
because it absorbs the case's own duration. With it:

| run | reading |
| --- | --- |
| a coded hold (`REPORT_CHECK` or `REPORT_CHECK_COLOUR`) | **174 + 6 * code** |
| the plain holds | **PASS 164, FAIL 144** |

`code` is 1 when the case passed, 1 + the first failing check otherwise, and
the checks are numbered from 1 (`gl_test_check` increments before it records,
so the first `expect_*` in a case is check 1). The 174 is `SETTLE` 100 plus
the 70 s coded base plus about 4 s of reset and boot before Linux starts
counting its own uptime.

That 4 s was measured, not assumed, and it matters: at 6 s per code step a
two-second error in the base is a third of a step, and three readings in one
session landed between two codes because the base had been guessed at 176.
The check is that codes come out whole. Measured 2026-09-21:

```
case 8   143 s plain   -> FAIL (144 predicted; PASS would be 164)
case 8   186 s coded   -> code 2.0 exactly: check 1
case 12  186 s coded   -> code 2.0 exactly: check 1
case 24  198 s coded   -> code 4.0 exactly: check 3
```

Against a base of 176 those last two decode as 1.67 and 3.67, which is how you
know 176 is wrong.

**`REPORT_CHECK_COLOUR=y` is the way to tell a pass from a failure when the
plain codes are close.** A pass still holds at code 1 (180 s) because the
colour only replaces the code when the case failed, while any failure holds at
code 32..39 -- 366 s to 408 s. Those cannot be confused, where code 1 (180)
and code 2 (186) can. So a colour run answers "did it pass?" and "what did it
read?" in one go, and is usually the better second run.

## A coded hold wants a fixed base

Three readings were misread in one session for the same reason: the number was
decoded against a base that was not pinned. `70 + 3 * EC` for a fatal and
`70 + 6 * code` for everything else can both claim the same total, and a
case's own duration (8 s, or 19 s once kicks hang) is comparable to the 6 s
step. `SETTLE` exists for exactly this and is not optional the moment a case's
duration can vary -- set it for single cases too, not just the set. And decode
against the sample's own `.config`: `gc_slint` sets
`GC_SAMPLE_HOLD_SECONDS=5`, so its pass is 64 s where the usual 60 would say
119.

## Results on the GC7000UL

**2026-09-21: the fragment-uniform bug is fixed and case 1 passes.** Two bugs,
neither of them in the Vivante register programming:

- `gen_shader_table.py` never emitted `blend_consts_off`, so every table entry
  claimed the blend constants sat at dword 0 and the GL core wrote
  `glBlendColor` over the first four dwords of every fragment uniform block on
  every draw. A vertex uniform lives in the other block, which is why one
  stage always worked and the other never did.
- This pixel engine has no RGBA8888 render format, so such a target is B,G,R,A
  in memory and `glReadPixels` handed back red and blue transposed. Symmetric
  colours passed, orange and cyan did not.

`src/vivante/VIVANTE_NOTES.md` has the full account, including the
three guesses spent before it and the host-side tool that ended it
(`build/run_cs_golden.sh`: the command stream Mesa produces for the same draw,
diffed against ours, no GPU needed).

| case | name | result | run |
| --- | --- | --- | --- |
| 1 | uniform | **PASS** | `kernel started 66 s after the jump` |
| 2 | texture | **PASS** | `kernel started 85 s after the jump` (code 1, with `REPORT_CHECK=y`) |
| 3 | depth | **PASS**, all 11 checks; the hung kicks are gone | `kernel started 180 s after the jump` (code 1, `REPORT_CHECK=y`, `SETTLE=100`, crash route) |
| 4 | cull | **PASS** | `kernel started 179 s after the jump` (code 1, `SETTLE=100`) |
| 5 | scissor | **PASS** | `kernel started 180 s after the jump` (code 1, `SETTLE=100`) |
| 6 | fbo | **PASS** | `kernel started 179 s after the jump` (code 1, `SETTLE=100`) |
| 8 | blend | **PASS** | `kernel started 179 s after the jump` (code 1, `REPORT_CHECK=y`), `164 s` plain, and `180 s` again after the `PS_MSAA_CONFIG` change. vivante-frag2, before yakogl `525b2d2`. The cause of its earlier failure was in the GL core, below |
| 9 | stencil | **PASS** | `kernel started 66 s after the jump` |
| 10 | prims | **PASS** | `kernel started 82 s after the jump` (code 1, with `REPORT_CHECK=y`) |
| 12 | stencil2 | **PASS** | `kernel started 180 s after the jump` (code 1, `REPORT_CHECK=y`) and `165 s` plain. vivante-frag2, before yakogl `525b2d2`, and it leans on depth/stencil, so it is one to re-read |
| 13 | zls | **PASS**, all 10 checks | `kernel started 180 s after the jump` (code 1, `REPORT_CHECK=y`, `SETTLE=100`, crash route). It read `186 s` (code 2, check 1) before the depth work and `240 s` (code 11, check 10) between the two fixes |
| 14 | api | **PASS** | `kernel started 180 s after the jump` (code 1, with `REPORT_CHECK=y` and `SETTLE=100`) |
| 15 | egl | **PASS** | read twice: vivante-vertex2, and `kernel started 180 s after the jump` (code 1, `REPORT_CHECK=y`, `SETTLE=100`) |
| 16 | msaa | **FAIL at check 6** | `kernel started 216 s after the jump` (code 7, `REPORT_CHECK=y`). vivante-frag2, before yakogl `525b2d2`. The 4x multisample path is switched off on that branch |
| 17 | ctx2 | **PASS** | `kernel started 180 s after the jump` (code 1, with `REPORT_CHECK=y` and `SETTLE=100`) |
| 18 | image | **PASS** | `kernel started 180 s after the jump` (code 1, with `REPORT_CHECK=y` and `SETTLE=100`) |
| 19 | es100 | **PASS** | `kernel started 180 s after the jump` (code 1, with `REPORT_CHECK=y` and `SETTLE=100`) |
| 20 | etc | **PASS** | `kernel started 179 s after the jump` (code 1) once the read-back alpha fix landed; before it, `259 s`/`258 s` (code 14, check 13) and `378 s` (code 34, green) |
| 21 | rgb565 | **PASS** | `kernel started 180 s after the jump` (code 1, `REPORT_CHECK=y`) and `164 s` plain. vivante-frag2, before yakogl `525b2d2` |
| 22 | vao | **PASS** | read twice: vivante-vertex2, and `kernel started 181 s after the jump` (code 1, `REPORT_CHECK=y`, `SETTLE=100`) |
| 23 | winding | **PASS** | `kernel started 180 s after the jump` (code 1, `REPORT_CHECK=y`) and `164 s` plain. vivante-frag2, before yakogl `525b2d2`, and it leans on depth/stencil, so it is one to re-read |
| 24 | compact | **PASS** | `kernel started 179 s after the jump` (code 1, `REPORT_CHECK=y`) and `165 s` plain. vivante-frag2, before yakogl `525b2d2` |
| 34 | unishare | **PASS** | `kernel started 180 s after the jump` (code 1, `REPORT_CHECK=y`) and `164 s` plain. vivante-frag2, before yakogl `525b2d2` |
| 40 | qtstrip | **PASS**, but see the note below | `kernel started 180 s after the jump` (code 1, `REPORT_CHECK=y`) twice, independently. vivante-frag2, before yakogl `525b2d2`, and it draws with depth, so it is one to re-read |
| 41 | es3cube | **PASS** | `kernel started 180 s after the jump` (code 1, with `REPORT_CHECK=y` and `SETTLE=100`) |
| 43 | es3vid | **PASS** | `kernel started 180 s after the jump` (code 1, with `REPORT_CHECK=y` and `SETTLE=100`) |
| 48 | es3vol | **PASS** | `kernel started 180 s after the jump` (code 1) once the case asked for LINEAR; before that `191 s` (code 3), check 2 setting NEAREST and wanting a blend |
| 53 | zprobe | **PASS**, both clear paths | `kernel started 180 s after the jump` (code 1, `REPORT_CHECK=y`, `SETTLE=100`). Its earlier value readings: `173 s` then `174 s` (code 0, the clear-alone path storing 0), then `408 s` (code 39) which settled the clip space |

Cases 1 and 9 were both failing before today, so the two fixes above carried
them.

**vivante-frag2's rows were all read before yakogl `525b2d2`**, and before
team-lead's instruction to build without the display driver, so they carry the
panel console as well. Four depth and clear-path fixes have landed since:
`f85791c`, `3864915`, `ea4d2f1` and `525b2d2`. Cases 12, 23 and 40 touch depth
or stencil and are the ones that could have moved in either direction; they are
marked for re-reading rather than restated as current.

**Case 40's plain readings were 18 s, twice, and nobody knows why.** Its two
coded readings are both 180 s and agree, which is why it is a pass. But with
`REPORT_CHECK` off it returned **18 s** both times -- not the 164/165 a plain
pass gives, and not the 9-11 s of a jump that never happened. It repeated per
configuration rather than at random, and the two builds differ only in a
Kconfig bool. Recorded as a warning rather than rounded into the nearest band,
because a number that fits no known path is the interesting kind.

**Case 3 passes as of 2026-09-22, and its hung kicks are gone.** It had been
failing with roughly eight kicks that never completed, check 3 reading black,
and a regression-set stall that made the whole set unreadable. Check 3 passing
is what settles the hangs rather than the timing: a kick that never completes
never lands its draw, which is why that check read black, and 19 s of hung
kicks would hide inside the 100 s settle without moving the reading.

Which fix carried it is **not established**. Two candidates, both defects
corrected on their own evidence: the early-depth fix, where
`RA_EARLY_DEPTH_TEST_ENABLE` was set while `PE_DEPTH_CONFIG`'s `EARLY_Z` was
not, so the two halves of the pipe disagreed about whether the depth test had
already happened from the first draw that enabled depth -- which is case 3's
frame 1 -- and the depth-write fix, since frame 2 turns the depth test off and
we were still writing depth, polluting frames 4 and 5. Attribution would need
a bisect and nobody has spent the board time on it.

Each row says how it was read, because a number without its base is not a
reading. The rows quoting `SETTLE=100` decode with the settled formula above,
`total = 174 + 6 * code`. The older rows predate it: they were taken with
`SETTLE=0`, where the case's own duration lands in the total, so they are
recorded as the raw return and the verdict that was drawn at the time rather
than re-decoded here. Re-read any of them with `SETTLE=100` if its exact code
ever matters.

**Case 13 (zls) fails at its first check, as of 2026-09-22.** That check is
`clear 0.5: mid quad passes`: the frame clears depth to 0.5 with
`glClearDepthf`, draws the mid quad at window z 0.4 so that it passes `LESS`,
then the far quad at 0.9 so that it does not. The centre should be blue. The
host side does not explain it -- the command stream, the clear quad's z and
the viewport z state all check out against Mesa (`yakogl tools/vivante/README.md`,
the depth-clear section) -- so the next fact wanted is what the pixel actually
reads, with `REPORT_CHECK_COLOUR=y`.

**Root cause found 2026-09-22, by case 53: the clear landed as 0.0, not 0.5.**
The clear quad carried `clear_depth * 2 - 1`, GL's window depth mapped back
into clip space. That is right for geometry, whose vertex shader Mesa's
compiler builds and whose output the rest of the pipe converts; it is wrong
for the clear quad, whose vertex shader is a single NOP that emits the
attribute untouched. Fixed in `gc_backend.c` by writing the value itself
(yakogl `8466fc6`). Case 13 has not been re-run yet, so it is not yet known
whether check 1 is the only thing that was wrong with it.

**Check 10 was a second, separate fault: `glDepthRangef` scaled the depth
clear.** `glClear` writes the clear value to the depth buffer as it stands and
the depth range does not scale it -- but this backend clears with a quad, and a
pending clear is carried out by the *next draw*, so the clear quad was
transformed by whatever range that draw was using. Case 13's last frame sets
`glDepthRangef(0, 0.3)` before the draw that takes the clear, so a 0.5 clear
stored 0.15 and both quads then failed `LESS` against it. Fixed by giving the
clear quad the identity range (yakogl `525b2d2`); `tools/vivante` mode 18 shows
it beside Mesa, which never meets it because it clears through the resolve
engine where no viewport transform applies.

That is the third fault of the same family in one night -- the depth range left
at `{0,0}`, the range leaking in from a neighbouring draw, and the clip-space
question -- and all three exist **because the clear is a quad**. Worth
remembering if the resolve-engine clear is ever revisited.

**Why check 1 took so long, which is the part worth keeping:** the two readings
agree at the ends of the range.

| clear | clip z | correct | unconverted | agree? |
| --- | --- | --- | --- | --- |
| 1.0 | +1.0 | 1.0 | 1.0 | yes |
| 0.5 | 0.0 | 0.5 | **0.0** | **no** |
| 0.0 | -1.0 | 0.0 | 0.0 | yes |

Every depth clear in the suite is the default 1.0 or case 3's 0.0, so every one
of them was right by coincidence. **Case 13 is the first test that clears
depth to an intermediate value**, which is exactly why it was the only one
failing, and why a test that clears to something other than an endpoint is
worth having.

The golden stream could never have found it: the value is four floats in a
vertex buffer, not a register, so the command stream was correct throughout.
See the "blocks the stream only points at" section of
`yakogl/tools/vivante/README.md`.

Ruled out before that, so that nobody spends a run re-treading them:

- **The clear quad is in the wrong depth space.** It is not. Mesa's own
  comment says this part's viewport z transform is 0..1 to 0..1 and folds the
  -1..1 conversion into the scale and offset it programs, which is exactly the
  `PA_VIEWPORT_SCALE_Z` 1.0 / `OFFSET_Z` 0.0 both drivers emit. Separately,
  `nir_lower_clip_halfz` is *not* applied for this GPU (`vs_need_z_div` is
  `model < 0x1000`; the model is 0x7000).
- **The hardware takes clip z as the stored depth, with no remapping.**
  Measured directly on 2026-09-22 and false: a clear whose quad carried 0.25
  stored 0.625, so the part converts -1..1 to 0..1 itself (case 53, code 39;
  identity would have read 15). Case 3's first check had already implied it --
  under identity its near triangle at NDC z -0.5 would clamp to the near end
  and never draw, and that check passes -- but the measurement is what
  settles it, and a root cause was built on the wrong answer in between.
- **`glClearDepthf` does not reach the backend.** It does: the value is
  captured at `glClear` time and the clear quad's vertex buffer carries
  `clear_depth * 2 - 1`, read back out of the memory dump as `0.0` for a 0.5
  clear and `1.0` for the default.
- **The draws use a non-zero first vertex and we ignore it.** `struct
  gles_draw` has no first-vertex field at all; the GL core folds the offset
  into the attribute pointers, which is why the backend's `first` is always
  zero and why the PowerVR backend works the same way.

That reading says which way the depth test went. It does not say what the
depth buffer held when it went that way, and those are different questions
with different fixes, so **case 53** answers the second one directly: it
clears depth to 0.5 and does nothing else, and `GC_GL_TEST_DEPTH_VALUE=y`
reports the value the hardware stored. **Expect 31 or 32** -- 0.5 of 16777215
is 8388607.5, so which side it rounds to decides the code, and reading 31 as a
failure because 32 was written down is the mistake this sentence exists to
prevent. Anything else names the fault without a further inference: 63 says
the clear value never reached the buffer, 0 says it reached it as the near end
of the range, 41 says the GPU wrote none of the surface at all.

63 is shared between "ignored" and "the surface is not uniform". To separate
them, rebuild with `-DCONFIG_GC_GL_TEST_ZPROBE_MILLI=250`: a quarter reads
back as 15 or 16, and only the first of those two explanations moves.

**Case 22 passes as of 2026-09-21, and the bug was in the test.** The case
asserted that a name from `glGenVertexArrays` is already a VAO. It is not: ES
3.0 6.1.10 and `OES_vertex_array_object` both say the call reserves names and
nothing more, and the object comes into being at its first bind, so
`glIsVertexArray` answers false until then --
`dEQP-GLES3.functional.negative_api.state.is_vertex_array` requires exactly
that. The check had been failing on every backend since YakoGL was made to
follow the rule. It now wants false before the bind, and a new check wants
true after it, which is the part that was worth testing and was not being
tested. The Vivante backend needed no change; everything else about VAO
record and restore was already right.

It cost one board run, because the eight state checks in the case were read on
the host first (below).


**Case 20 fails at check 13 as of 2026-09-22, and the ETC texture path is not
why.** Its checks are 1-3 format queries, **4-11 the eight ETC1 blocks**,
**12-15 the four ETC2 RGBA8 EAC blocks**, 16-17 the error cases. Two runs, 259 s
and 258 s, both decode to code 14, so the first failure is check 13 -- which
means all of ETC1 passes on silicon, along with ETC2 block 0.

The four ETC2 blocks differ **only in alpha**: 255, 128, 64, 0. Block 0 wants
255 and passes; block 1 wants 128 and is the first failure. A third run with
`REPORT_CHECK_COLOUR=y` returned `378 s`, code 34, **green** -- the colour that
block is supposed to be. So the RGB decode is right and the alpha is not
surviving.

**Fixed, and confirmed on silicon: `kernel started 179 s`, code 1, case 20 passes.**
That is the first board reading of yakogl `9e33d7b` (frag2's `740907d`, cherry-picked to main), whose own message recorded it as unverified. The prediction it tested was exact: checks 13, 14 and 15 would pass and 16 and 17 already passed on the host, so the case would pass outright, and if it had stopped at 14 or 15 the EAC decode would have been implicated. It did not.

The cause was in `glReadPixels`, not in the texture: `gl_framebuffer.c`'s BGRA
branch writes `o[i + 3] = 0xff` unconditionally, and its `bgra` flag covers
`GLES_SURF_RGBA8888` on a backend whose pixel engine stores B,G,R,A as well as
the XRGB scan-out surface it was written for. Every RGBA8888 readback therefore
comes back opaque, and check 12 passed only because 255 was the wanted value.
Found by vivante-frag2 on case 8. Note the *other* `= 0xff` a few lines above,
in the RGB565-to-RGBA expansion, is correct and must stay: RGB565 has no alpha.

**Case 15 passes as of 2026-09-21, first try, with no change to anything.** The
case is EGL rather than GL -- initialise, choose a config, refuse an 8x MSAA
one, a pbuffer and its queries, two contexts, `eglGetProcAddress`,
`eglSwapBuffers` -- and the Vivante backend already satisfied all of it. Its
thirteen state checks had already passed on the host build, so the board run
was only ever going to answer for the pixels, and they were right too.

**Case 2 passes as of 2026-09-21**, after two bugs in how a mip chain is laid
out for this part, both found by asking the board which check failed rather
than which case:

- the descriptor names all sixteen level addresses but carries one
  `LINEAR_STRIDE`, so every level's rows have to be that far apart; each level
  was packed at its own width, and level 1's second row fell in level 2
  (check 22, `216 s` -> `222 s`);
- a level is fetched from a 64-byte boundary, and level 2 of a 4x4 chain sat
  at `+0x60`, which rounds down to the `+0x40` that is level 1 -- the board
  reported the green that says so (check 23, code 34, `283 s` -> `85 s`).

**Two findings from the other agents that are not Vivante-specific**, recorded
here because the next reader will meet them through this suite:

- **Case 8's root cause was in the GL core, not the backend**: `glReadPixels`
  wrote 0xff into the alpha byte of an RGBA8888 render target, so any test
  reading back alpha saw 255 whatever had been drawn. It is shared code, so it
  was wrong on the PowerVR too and simply had nothing reading alpha to notice.
- **The stencil face swap is `front_cw != flip_y`**, by board measurement.
  That is the same shape as the cull mirror in case 4 below -- the rasteriser
  runs y down, so which face is "front" depends on whether the target was
  flipped into that space -- and the two should be read together.

**Case 5 passes as of 2026-09-21**, and what that does *not* say is worth
recording: the scissor margin was corrected shortly before
(`SE_SCISSOR_RIGHT` 0x0040ffff -> 0x00401119, Mesa's
`ETNA_SE_SCISSOR_MARGIN_RIGHT`; the clip margin stays 0xffff and ours had been
using it for both). Case 5 had no reading before that change, so its pass
cannot be credited to it -- only that the corrected margin does not break the
case that drives `glScissor`. The fix stands on matching Mesa, not on this.

**Cases 4 and 6 pass as of 2026-09-21.** Case 6 needed nothing: its two halves
were checked on the host first -- the FBO draw state matches Mesa register for
register, and sampling back from a render target gets `A8R8G8B8` where an
uploaded texture gets `A8B8G8R8` -- and it passed as it stood.

Case 4 took two goes at the same bug and is worth reading as one. The backend
mirrored the culled winding for a flipped target; that culled a triangle which
should have been drawn, and the board read the pixel as black. Removing the
mirror carried checks 1 to 4 -- all four windings crossed with both cull faces
-- and left check 5, the same test against an FBO, which never took the mirror
in either direction and so could not be explained by either having it or not
having it.

Putting *both* harnesses on an FBO target settled it. For the same GL calls,
`ours PA_CONFIG 0x00412100` (CULL_FACE_CW) against `Mesa 0x00412200`
(CULL_FACE_CCW). The rasteriser runs y down: the window surface's viewport
flip already lands it in that space, while an FBO keeps GL's y-up and is
upside down to it. So the mirror is real and belongs on the **un-flipped**
target. Until then every harness mode compared our window surface against
Mesa's FBO, and the y-flip differences that came with that were noise the eye
learned to skip.

**Case 10 passes as of 2026-09-21**, on one bug in the points, found by
reading Mesa rather than the board. `SH_SPECIALS`' `VS_PSIZE_OUT` is the
offset of `gl_PointSize` in the vertex output stream, in components, and it
was computed from the output count *after* the point size had been counted in
-- naming component 12 where `points.vert` puts it at 8, so the hardware read
whatever follows the stream as the size of every point. Mesa draws exactly
this distinction and only here: its `VS_OUTPUT_COUNT` excludes the point size
and feeds `SH_SPECIALS`, while `VS_OUTPUT_COUNT_PSIZE` includes it and feeds
the other four registers.

That one was settled on the host with `build/run_cs_ours.sh 7` against
`build/run_cs_golden.sh 7` -- `0x7f7f7f0c` against Mesa's `0x7f7f7f08` -- so
it cost one confirming board run rather than a search. Everything else about
the primitives already matched: the `DRAW_INSTANCED` encoding, the
primitive-type codes, the line width in `PA_LINE_WIDTH` and both
`PA_WIDE_LINE_WIDTH` registers, and `PA_CONFIG`'s `WIDE_LINE`.

Case 4 is not a result, it is a missing one. Nine seconds is what the table
above calls "the jump never happened", and three attempts in a row all read
9 s while cases 9 and 10 ran normally between them, which looked like a
per-case cause rather than three coincidences.

**That inference has a counterexample as of 2026-09-21.** Over nine runs that
session, three came back in the 9-11 s band -- case 10, case 11, and the
regression set -- and every one of them read normally on the next attempt. So
the flake is about one run in three, it is not per-case, and three of them in
a row is roughly a one-in-thirty coincidence rather than evidence of
something particular to case 4. Case 4 deserves the table's own advice: run it
again.

Budget for it. A session that needs ten readings will spend thirteen runs
getting them, and a result that only appears once is not yet a result.

**The regression set finishes, 2026-09-21, and the stall was not its own bug.**
With case 3 left out it returns `kernel started 264 s after the jump`, which is
`SETTLE` + 70 + 6 * 15: **fifteen cases pass** and the suite completes inside a
hundred seconds, where it used to spend four hundred and not reach case 11.

The stall was case 3's cascade. A kick that never completes costs two seconds
in `gc_render_wait` and leaks its hardware event for good, so after thirty of
them every later submit pays a second in `gc_event_alloc` as well; one case
hanging kicks slowed everything after it. Two other explanations were measured
and are wrong: the panel console is not the cost (`540 s` with it off against
`539 s` with it on), and case 11 is not simply a slow case despite its 300
draws (`81 s` on its own, a pass). `CONFIG_GC_GL_TEST_SKIP` takes a decimal
list and counts a skipped case as neither a pass nor a failure.

The first failure among the remaining cases is **case 4**.

## A probe can change the path it is measuring

Case 53 was written to isolate a depth clear so the stored value could be read
back. Isolating it moved the clear onto a different code path: a clear with no
draw behind it is carried out by `gl_flush_pending_clear`, which builds a draw
of its own, and that draw was leaving `depth_range` at `{0, 0}`. The Vivante
backend derives `PA_VIEWPORT_SCALE_Z = far - near` from it, so the scale was
zero and every clear stored 0 -- while the path case 13 actually uses, where a
draw carries the clear, was correct all along.

So two board readings measured a real bug, and neither measured the case they
were taken for. **When a probe isolates the thing under test, check that it has
not isolated it onto different code.** Case 53 now exercises both paths and
checks both, so the pair cannot drift apart again:

| check | path |
| --- | --- |
| 1, 2 | the clear is carried out alone, then drawn over |
| 3, 4 | a draw follows the clear and takes it along |

Its last act is still a clear with no draw, so `GC_GL_TEST_DEPTH_VALUE` has a
uniform surface to read. That reading describes the first path only; the second
has checks and no reading.

### Why these are hard to notice

Four tools misled somebody on 2026-09-21/22: a mode list that had drifted, a
harness that drew mode 1 when asked for a mode it did not have, this probe
reporting "no majority" as though it were a depth value, and
`run_gl_test_host.sh` building a different agent's worktree and reporting PASS
for a case that had none of those checks in it.

What they share, in vivante-vertex2's words, is that **the wrong answer is
well-formed**: a descriptor with plausible fields, a PASS line in the right
format, a depth value inside its range, a stream that decodes cleanly. Nothing
in the output marks it as not-an-answer, so no amount of reading the output
carefully will catch it. Each was caught by an outside fact -- a mode known not
to exist, a PASS known to be impossible without a GPU, a board reading that
refused to move.

So the defences are: have the tool **say what it did** (which tree, which mode,
which path), give it an **exit code** rather than a phrase for a script to
test, and keep one **authoritative list** rather than a copy that can drift.
And when a check reports success, ask what it would have printed had the thing
been broken -- one of these was found only because its own verification grepped
for wording the tool never emits, and so passed everything.

## The host build: the checks that are not pixels

A board run costs minutes and the board is shared, but a lot of what a case
checks is not a pixel: an extension string, a query, an object name, an EGL
return. Those need no GPU.

`build/run_gl_test_host.sh <case>` compiles the same `gl_test.c` against the
Linux OSAL, which keeps the command stream instead of executing it, and prints
every check by name in about a second:

```
gl_test:   GL_OES_vertex_array_object advertised = 1  want 1  ok  [check 1]
gl_test:   generated name is not a VAO yet = 0  want 0  ok  [check 3]
gl_test:   vao: centroid        (32,40) = 0x00000000  want 0xff595452  MISMATCH  [check 6]
```

**Every check that reads a pixel fails here, and that is not news.** Nothing is
drawn, so `glReadPixels` comes back all zeroes. The harness narrows a case
before a board run; it does not replace one.

It builds the *PowerVR* backend, which makes it the "did I break PowerVR"
check as well -- the Vivante work edits `src/gl_draw.c`, `src/gl_texture.c` and
`src/backend.h`, and those are shared with the other GPU.

Read 2026-09-21, one second per case:

| case | what the host settles |
| --- | --- |
| 14 api | checks 2-3 pass; 1 and 4-11 are pixels |
| 15 egl | all thirteen EGL checks pass; the rest are pixels |
| 17 ctx2 | all eleven state checks pass; the rest are pixels |
| 19 es100 | links, locations and queries pass; the rest are pixels |
| 22 vao | all eight state checks pass -- and check 3 was the bug |

**One false lead to know about.** Case 18's check 4, "image from native
buffer", fails here and means nothing: the native-pixmap path needs
`g_osal->map_phys`, and `src/osal/linux.c` sets it to `NULL`, so the call is
refused for a reason that does not exist on the board (`osal/zephyr_gc.c`
provides it). Do not chase it on the host. **Confirmed 2026-09-22: case 18 passes on the board**, check 4 included, so that host failure was purely the missing `map_phys` and nothing else.

**Cases 8, 12 and 23 pass as of 2026-09-22**, on two bugs, neither of them in
the Vivante register programming that the case names point at.

- **Case 8 blend** was one channel wrong, and the colour hold said which. The
  first check read hold code 38 -- red and green at or above half, blue below
  -- which is the signature of the value it wanted (`0xbf008080`). So the
  blend was right in all three colour channels and the check still failed.
  `glReadPixels` was writing `0xff` over the fourth byte of every B,G,R,A
  target. That is right for a scan-out XRGB8888 surface, whose fourth byte is
  an X, and wrong for an RGBA8888 one, whose fourth byte is the alpha the
  pixel engine blended. It hid because every case that passes here wants alpha
  `0xff`; case 8 is the first to read back a blended one.
- **Cases 12 and 23** are the two-sided stencil, and they turn on a hardware
  fact worth knowing: **the cull test and the face test read different
  windings.** Case 4 measured the rasteriser culling on the clip-space
  winding, before the viewport transform. The pixel engine does not: with the
  face swap taken to be `glFrontFace` alone, case 12's first check -- a
  GL-front-facing triangle that must come out red -- read blue, the back
  reference (`372 s`, code 33). So a top-down window needs the flip in the
  stencil swap (`front_cw != flip_y`) and no mirror at all on the cull face.
  Mesa is no guide: it renders bottom-up and never flips, so its stream gives
  the unflipped answer for both units.

**Case 24 compact passes as of 2026-09-22, and its premise was wrong.** The
case is named for sampler compaction: it declares two samplers, reads only the
second, and expects the compiler to drop binding 0 and compact binding 1 down
to register 0. **This toolchain does not compact.** The instruction says so --
`texture.frag` (binding 0) compiles to `0x07811018`, `compact.frag` (binding
1) to `0x0f811018`, and `texture2.frag`, which samples both, to `0x07821018`
then `0x0f811018`. Bit 27 of the first word is the sampler index, and
`compact.frag`'s single load has it set: the surviving sampler still reads
register 1.

So the descriptor belongs in slot 1, and the two fixes that put it in slot 0
-- one reading `etna_shc`'s `"unit"` field, which reports 0 for every sampler,
and one deriving the dense renumbering by hand -- both left the shader
sampling an empty slot, which reads black (`366 s`, code 32). What *is* the
compiler's to say is which samplers survive, and that part stands: a dropped
sampler is left out of the table, so `glGetUniformLocation` answers -1 and the
case's first check passes. Both reflection checks passed throughout; only the
pixel was wrong.

Ruled out along the way, so nobody re-spends them: the descriptor block (the
raw 256 bytes differ from Mesa's only in the format, addressing mode, stride
and tiled-only HALIGN that `VIVANTE_NOTES.md` already accounts for, and both
`LOG_SIZE` words are zero on both sides); the stride (case 2 passes with a 2x2
texture, whose linear stride is 8 bytes, the same as this 1x1's); and the wrap
mode (case 24 takes GL's default repeat where every other textured case asks
for clamp, which looked decisive and was not -- the fix for it is still in,
because S and T were not wrapping independently, but it did not move the
case).

Case 2 was re-run afterwards and still passes (`179 s`, code 1): the shader
table was regenerated three times getting here and the two-texture-unit check
is the one that would have caught a bad binding.

**Case 16 msaa fails at check 6 and nowhere else**, which says exactly what is
missing. Its first five checks pass: a 4-sample config is offered, it reports
`EGL_SAMPLES` 4, `GL_SAMPLES` comes back 4, and the triangle renders and reads
back correctly inside and outside. Only `x4 edge: half covered` fails -- the
pixel on the diagonal, where two of four samples should land either side, is
coming back fully covered or empty because the surface is single-sampled.

So everything around multisampling already works and the multisampling itself
does not. On this part 4x is 2x2: the surface is allocated twice as wide and
twice as tall and its strides double, while the viewport stays in target
pixels -- the rasteriser still makes one pixel per target pixel and the
hardware spreads that pixel's four samples across the larger surface.
`RA_MULTISAMPLE_UNK00E10[0..2]` and a twelve-entry `RA_CENTROID_TABLE` put the
four samples on a rotated grid, which is not optional: on an ordered 2x2 grid
the two subpixels on the line x = y are a fill-rule tie, so the edge comes out
a quarter covered and check 6 fails just as it does single-sampled. The
resolve is the RS, a block separate from the 3D pipe driven from the same
stream, with DOWNSAMPLE_X and DOWNSAMPLE_Y making the copy an average.

**All of that is written, and switched off, because the 2x2 rasterisation does
not work yet.** Three readings, all with the case's first five checks passing:

| the 4x path | reading |
| --- | --- |
| off, as now | `216 s`, check 6: the edge is not antialiased |
| on, through the scratch, but rendering *single-sampled* and copying out 1:1 | `216 s`, check 6: the baseline picture |
| on, 2x2 with a downsampling RS | `204 s`, check 4; the colour hold reads black (`365 s`) |

The middle row is the useful one. **The plumbing works.** The scratch
allocation, redirecting the pixel engine's colour into it, the resolve engine
copying it back and the fence ordering together deliver exactly the picture
the ordinary path does. So the resolve is not the problem, and what remains is
only what changes when the rasterisation itself goes to 2x2: the sample count
in `GL_MULTI_SAMPLE_CONFIG`, the opaque `RA_MULTISAMPLE_UNK00E10` and
`RA_CENTROID_TABLE` words, the doubled colour stride, the depth surface at
twice the size, and the RS downsample bits. The RA words are the first place
to look -- they are copied from Mesa and nothing is known about their
encoding.

Already ruled out, so nobody re-spends them: scaling the viewport (a wrong
guess -- multisampling scales the *surface*, not the geometry, which is where
Mesa's MSAA scale reaches and stops); `PS_MSAA_CONFIG`, which was genuinely
wrong for other reasons and is now Mesa's value; and one real bug of mine, the
sample count disagreeing with the surface when the backend declines to
multisample a draw the application asked to multisample. That last one made
the switch itself look broken, and chasing *that* is what produced the middle
row -- an earlier reading claiming the resolve engine never wrote the target
was measuring the bug, not the RS, and has been retracted.

The switch stays off because a frame that renders multisampled and does not
come out is worse than one that never multisampled.

**Cases 21, 34 and 40 pass as of 2026-09-21, with no driver change needed.**
Each was read twice, which is what the flake budget above asks for: the coded
hold gives 180 s (code 1) and the plain hold 164 s, and those are the two
numbers a pass has under `SETTLE=100`.

- **34 unishare** -- two draws in one kick with the uniform changed between
  them. This is the PowerVR bug that made a Slint dashboard draw every element
  in the last colour, and it does not exist here: uniforms are written into
  the command stream as `LOAD_STATE` words per draw, so each draw already
  carries its own. The golden harness says the same thing, mode 14: our two
  draws send `0x3f800000 0x3f000000 0 0x3f800000` and then
  `0 0x3f800000 0x3f800000 0x3f800000`, which is Mesa's stream exactly.
- **40 qtstrip** -- Qt Quick's merged strip, 34 indices with degenerate
  triangles, depth LESS, and a third attribute stream carrying each element's
  z. Also a PowerVR-specific bug (a USC task sized from the vertex *output*
  count lost the ninth input past the first task), and also absent here. Both
  the client-array and the index-buffer draw pass.
- **21 rgb565** -- a 16-bit window surface end to end, including the raw memory
  check that red lands in bits 15..11, and a second frame with no clear that
  has to keep the first one.

**Case 40 has two coded readings and no usable plain one, which is worth a
warning of its own.** Its coded runs give 180 s twice, so the pass is read
twice as this file asks. Its *plain* runs returned exactly 18 s, twice. That
is neither the 164/144 the plain holds give nor the 9-11 s the table calls a
no-jump, and it repeated per configuration rather than at random, so it is not
the ordinary flake: the two builds differ only in `REPORT_CHECK`, a Kconfig
bool. Unexplained, and a reminder that a reading outside every band in the
table above is not a verdict to be rounded to the nearest one.

## The shader table

**2026-09-21: all forty shader sources in `gl_test_shaders.h` are in the
Vivante table**, matched by SHA-256. The twelve that were missing were all
GLSL ES 3.00 (`array.frag`, `cube.vert/frag`, `dynidx.frag`, `facing.frag`,
`hdrsample.frag`, `inst.vert`, `inst2.vert`, `texsize.frag`, `vcolor.frag`,
`vid.vert`, `vol.frag`); `tools/vivante/glsl2vivante.py` has grown an ES 3.00
path and the table went from 138 entries to 304. Cases 41-43 and 46-49 will
now get as far as running rather than failing on a shader that is not there --
which is not the same as passing, and none of them has a reading yet.
