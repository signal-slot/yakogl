<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 images S01-S16 at 8990a5cc

Run 2026-11-07 09:30-10:53 UTC on the Raspberry Pi 4 (V3D 4.2, Zephyr
af17c0c8, tryboot), item pi4-deqp-b of plan #308 (issue #2). The sixteen
images are rows S01-S16 of `tools/cts/pi4-gles3-tree.txt`
(`dEQP-GLES3.functional.shaders.*` from `arrays` to `negative`), built with
`tools/cts/pi4-deqp-images.sh tools/cts/pi4-gles3-tree.txt <img-dir> S01 ...
S16` from origin/main `8990a5cc` (`8990a5ccbba6c0cff99cd2da7f462568be4f24c4`,
a clean worktree of that commit) with the gles3 stage 1 of
`docs/es3/board-recipe-pi4.md` section 2 (VK-GL-CTS 93bca01, Zephyr SDK
1.0.1, built for this run in 4 min 32 s). Every image built `rc=0` with the
`--gles3` conformance line, at 161,844 or 161,848 KB of the 256 MiB window
(61.74 %).

## Result

9,461 mustpass cases in the sixteen images; 8,319 with a result: **8,228
pass, 86 fail, 0 NotSupported, 0 QualityWarning, 5 ResourceError, 1,142 never
began** (dEQP aborted the six `builtin_functions.precision` images at their
first case). 0 GPU recoveries, no job hung, no MMU fault, no reset before the
end of any run. Per image (`<LABEL>.tally.txt`; "begun" is the image's own
counter over its whole filter, the other columns are the mustpass view):

| image | mustpass | begun | pass | fail | n.s. | QW | other | none | run | verdict |
|---|---|---|---|---|---|---|---|---|---|---|
| S01 | 861 | 1309 | 861 | 0 | 0 | 0 | 0 | 0 | 46.9 s | DONE |
| S02 | 392 | 392 | 392 | 0 | 0 | 0 | 0 | 0 | 12.6 s | DONE, all passed |
| S03 | 192 | 1 | 0 | 0 | 0 | 0 | 1 | 191 | 0.2 s | STOPPED EARLY by dEQP at #0 |
| S04 | 201 | 1 | 0 | 0 | 0 | 0 | 1 | 200 | 0.2 s | STOPPED EARLY by dEQP at #0 |
| S05 | 202 | 1 | 0 | 0 | 0 | 0 | 1 | 201 | 0.2 s | STOPPED EARLY by dEQP at #0 |
| S06 | 204 | 1 | 0 | 0 | 0 | 0 | 1 | 203 | 0.3 s | STOPPED EARLY by dEQP at #0 |
| S07 | 206 | 1 | 0 | 0 | 0 | 0 | 0 | 206 | 0.2 s | STOPPED EARLY by dEQP at #0 |
| S08 | 142 | 1 | 0 | 0 | 0 | 0 | 1 | 141 | 0.2 s | STOPPED EARLY by dEQP at #0 |
| S09 | 1156 | 1156 | 1156 | 0 | 0 | 0 | 0 | 0 | 43.2 s | DONE, all passed |
| S10 | 690 | 1032 | 605 | 85 | 0 | 0 | 0 | 0 | 39.3 s | DONE |
| S11 | 756 | 756 | 756 | 0 | 0 | 0 | 0 | 0 | 39.2 s | DONE, all passed |
| S12 | 1216 | 1295 | 1216 | 0 | 0 | 0 | 0 | 0 | 55.1 s | DONE, all passed |
| S13 | 957 | 957 | 957 | 0 | 0 | 0 | 0 | 0 | 42.3 s | DONE, all passed |
| S14 | 594 | 594 | 594 | 0 | 0 | 0 | 0 | 0 | 25.5 s | DONE, all passed |
| S15 | 972 | 972 | 972 | 0 | 0 | 0 | 0 | 0 | 40.8 s | DONE, all passed |
| S16 | 720 | 720 | 719 | 1 | 0 | 0 | 0 | 0 | 31.7 s | DONE |
| total | 9461 | 9189 | 8228 | 86 | 0 | 0 | 5 | 1142 | | |

"begun" above "mustpass" (S01, S10, S12) is the filter matching cases outside
the mustpass list; they are left out of the other columns (S01's two `F` in
its end line are `constant_expressions.other.complex_{fragment,vertex}`,
outside the list: "'case' : two case labels have the same value"). "none" is
the cases of an aborted image that never began. S07's one case,
`precision.reflect.highp_fragment.scalar`, is outside the list, so S07 has no
mustpass result at all. The `--table` view over all sixteen consoles is
`table.txt` (the same TOTAL row, then every non-Pass case by name).

### Why S03-S08 stopped early

All six `builtin_functions.precision` images ended at their first case, the
same way. Each case's first draw goes to a framebuffer the V3D back end
refuses (`yakogl/v3d: draw refused: a target larger than the tile memory is
sized for (2048x2048), or empty`, then `yakogl: glDrawArrays was not recorded
(-22)`), the GL core reports `GL_OUT_OF_MEMORY` (`GL error 0x505 in
glDrawArrays`), dEQP records a ResourceError (`Error in draw: glGetError()
returned GL_OUT_OF_MEMORY at glsShaderExecUtil.cpp:684`) and ends the run
(`Test run was ABORTED!`, `complete=0`):

| image | the one case (#0) | mustpass cases without a result |
|---|---|---|
| S03 | `precision.abs.highp_fragment.scalar` | 191 |
| S04 | `precision.cosh.highp_fragment.scalar` | 200 |
| S05 | `precision.fract.highp_fragment.scalar` | 201 |
| S06 | `precision.mix.highp_fragment.scalar` | 203 |
| S07 | `precision.reflect.highp_fragment.scalar` | 206 |
| S08 | `precision.sub.highp_fragment.scalar` | 141 |

`glsShaderExecUtil.cpp` runs a fragment-stage precision case through its
`FragmentOutExecutor`, which renders into a framebuffer object of its own;
the refusal says that target is either empty or larger than 2048x2048 for the
V3D's tile memory. Which of the two, and the target's size, was not read here
(no code change in this item). Until the back end takes that target,
the `precision` group (1,147 mustpass cases) has no result on the Pi 4;
splitting the images does not help, every image fails at its first case.

### The other non-Pass cases, by cause

- `derivate` (S10): 70 mustpass Fail, every `dfdx`/`dfdy`/`fwidth` case of
  the `texture.*` subgroups in the list (the non-texture `derivate` cases
  pass):
  - 55 "Fail" from the result check: `texture.basic` (15),
    `texture.float_fastest` and `texture.float_nicest` (40).
  - 15 `texture.msaa4`: "Runtime check failed:
    'gl.checkFramebufferStatus(GL_FRAMEBUFFER) == GL_FRAMEBUFFER_COMPLETE'"
    (no 4-sample FBO, as M1's NotSupported in the first run). Outside the
    mustpass, the `fbo_msaa2`, `fbo_msaa4` and `{fastest,nicest}.fbo_msaa4`
    cases fail the same way.
- `fragdepth` (S10): 15 mustpass Fail (all 8 `compare.*` and 7 `write.*`),
  "Compile failed": the V3D QPU compiler refuses `gl_FragDepth` (`yakogl:
  online shaderc: the code generator refused this program (vertex +
  fragment): Program(NotYet("gl_FragDepth"))`).
- `metamorphic` (S16): 1 Fail `synthetic.variant_3`: `the code generator
  refused this program (vertex + fragment): Program(Select(OutOfScratch))`.
- `builtin_functions.precision` (S03-S08): 5 ResourceError (the first case of
  S03-S06 and S08; S07's is outside the mustpass), above.

Groups that pass in full: `arrays`, `builtin_variable`, `conditionals`,
`constant_expressions`, `constants` (S01, 861), `builtin_functions.common`
and `pack_unpack` (S02, 392), `conversions` (S09, 1,156), `declarations`,
`derivate` outside `texture.*`, `discard`, `fragdata`, `function(s)` (S10,
605), `indexing` (S11, 756), `invalid_implicit_conversions`, `invariance`,
`keywords`, `large_constant_arrays`, `linkage` (S12, 1,216), `loops` and
`matrix.*` (S13-S16), `negative` (S16).

## Method

As in `2026-10-31-pi4-gles3-a`: the Pi 4's UART bridge takes one client at a
time and the leased serial console (an MCP server connected forwards) holds
it, so `tools/board.sh pi4 tree` could not capture the console itself. The
board steps ran by hand in `board.sh`'s order, from the origin/main worktree:

1. `tools/board.sh --dry-run pi4 run <zephyr.bin> <dir> <LABEL>`: the local
   image checks (AArch64 header, the 256 MiB window, the PM watchdog marker).
   Every image passed (`rc 0`); nothing was bypassed.
2. Prechecks over ssh as `common_prechecks` and `pi4_prechecks`: `sudo -n`,
   boot_id, `who` (no session from another machine; this run's own ssh login
   arrives through connected's port forward), `/boot/firmware` free space at
   least twice the image, no leftover `yakogl-*.bin`, no `config-linux.txt`.
3. Staging as `pi4_stage`: the image to `/boot/firmware/yakogl-<LABEL>.bin`
   through `mktemp -d` and `install -m 0755`, `tryboot.txt` written with
   `pi4_config`'s lines (`kernel=`, `arm_64bit=1`, `enable_uart=1`,
   `uart_2ndstage=1`, the panel's `hdmi_group`/`hdmi_mode`/`hdmi_timings`
   from the environment), both read back (sha256 and text) before the reboot.
4. `sudo reboot "0 tryboot"`; the console read through the leased serial
   console (`serial_expect` on the end line `v3d_deqp: build 8990a5cc
   (DONE|INCOMPLETE|STOPPED)`, then `serial_read` of the run, which the
   session stored to disk as JSON; the console files were cut from those with
   a script, no copy by hand).
5. Linux back (a new boot_id, 297-304 s after the reboot every time: the
   image's 240 s lifetime, the PM watchdog reset, the Linux boot), then
   `pi4_undo`: the staged image removed and `tryboot.txt` removed (there was
   none before), read back. Every cycle read back as before; at the end
   `/boot/firmware` holds no `yakogl-*.bin` and no `tryboot.txt`
   (`config.txt` untouched throughout).

One GL client at a time: each image ran alone, the next one staged only after
Linux was back and the previous one's files were gone. The consoles hold no
address: the cut starts after Linux's `reboot: Restarting system` and ends at
the image's end line, before Linux boots again (its getty banner prints the
board's addresses); the only dotted number left is the CTS version
`1.4.6.2`.

## Files

Every `.txt` starts with the three `#` header lines the other log text files
carry (SPDX, then a title); the tally reads past them.

- `<LABEL>.console.txt`: the image's console from the firmware's first line
  of its boot to the `v3d_deqp: build 8990a5cc ...` end line (the watchdog
  reset prints nothing after it), CRs and NULs dropped, one line per row, no
  time stamps.
- `<LABEL>.tally.txt`: `python3 tools/cts/deqp-uart-tally.py --mustpass
  tools/cts/gles3-mustpass.txt <LABEL>.console.txt` (the script of
  `8990a5cc`).
- `table.txt`: the same with `--table` over the sixteen consoles.
- `cycles.txt`: per image the sha256 of `zephyr.bin`, the boot_id before,
  the reboot time, the seconds until Linux was back, the clean-up read-back,
  the `cases in` line and the end line.
