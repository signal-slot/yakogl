<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# T6 sweep at main d9efa418: panthor-gltest --sweep x2

Board: NanoPC-T6 (RK3588, Mali-G610 MC4, panthor, kernel
6.18.55-current-rockchip64), the board's own Armbian, not Zephyr. boot_id
unchanged over both runs (`ad1ec39d-…`): before `ad1ec39d-d5a1-4be3-a289-3cf5fcb59d6d`,
after `ad1ec39d-d5a1-4be3-a289-3cf5fcb59d6d` (each run's own before/after
and `tools/cts/mali-board-state.sh` around both). 2026-11-26, about 00:02
UTC (run 1) and 00:03 UTC (run 2); the board had been up since 2026-11-24.

Code: origin/main `d9efa418`, nothing on top, in a clean tree (`git describe
--always --dirty` gives `d9efa418`). The binary carried
`yakogl-build=d9efa418` and printed `panthor-gltest: build d9efa418`.
Reference record: `2026-11-t6-sweep-1a22bd38` (main `1a22bd38e`, 2026-11-12).

How it ran: `tools/mali/panthor/run-on-board.sh panthor-gltest --sweep`
twice, with the cross toolchain of `tools/cts/deps-env.sh --build --board
nanopc_t6 MALI_DEQP_TOOLCHAIN` first on PATH. The script took the board
lock, cross-built the static aarch64 binary, checked the build stamp, read
boot_id before and after, and captured `dmesg -w` between its run markers.
`tools/cts/mali-board-state.sh snapshot` before the first run and after the
second, then `compare`: /boot unchanged (419 files), modules unchanged (100),
U-Boot env not readable (not compared), boot_id unchanged. Raw logs and
dmesg captures stay out of the tree (records/nanopc-t6/).

Kernel lines: 0 kernel messages between the run markers and 0 in the 1 s
after the end marker, in both runs. The boot's `dmesg | grep -i panthor`
was the same before and after apart from the four run markers: the 8
probe lines of the boot (firmware, `Initialized panthor 1.5.0`), no fault,
reset or timeout line.

## 1. Sweep, two runs

`sweep-1.verdicts.txt` and `sweep-2.verdicts.txt`: the 256 verdict lines
and the summary of each run, identical (`cmp` exits 0; sha256
`bddbd1beb358620a8858fe7eac11899614186b6efaefd99dbfbfd41d0d2a3a35`).

    panthor-gltest: 256 case(s) run: FAIL 1, NO-CASE 53, PASS 185, REFUSED 9, SKIP 8
    panthor-gltest: 256 case(s) run: FAIL 1, NO-CASE 53, PASS 185, REFUSED 9, SKIP 8

Tally per verdict class, against 1a22bd38 (`FAIL 3, NO-CASE 73, PASS 165,
REFUSED 6, SKIP 9`):

| Class | 1a22bd38 | d9efa418 | Delta |
|---|---|---|---|
| PASS | 165 | 185 | +20 |
| REFUSED | 6 | 9 | +3 |
| FAIL | 3 | 1 | -2 |
| SKIP | 9 | 8 | -1 |
| NO-CASE | 73 | 53 | -20 |
| FAULT | 0 | 0 | 0 |

## 2. The cases that are not PASS, NO-CASE or SKIP

| Case | Verdict | Refusal or failure named | Since 1a22bd38 |
|---|---|---|---|
| 77 transc | REFUSED compile | fragment `arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet` | unchanged |
| 106 mrt3 | REFUSED draw | `several colour targets in a frame that does not start from a whole colour clear are not built yet` | unchanged |
| 128 uniidxj | REFUSED compile | fragment `a fragment varying load or conversion of a form no reference draws (a colour lane of unmeasured precision beside a mediump varying, an aligned pair converted apart, lanes converted out of Mesa's order) is not lowered yet` | was PASS |
| 163 drawbuf0 | REFUSED draw | `a draw whose first draw buffer is GL_NONE is not built yet` | unchanged |
| 226 negapi_uniformui, 227 negapi_uniformuiv | REFUSED compile | fragment `a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) is not lowered yet` | unchanged |
| 228 negapi_readpix_fmt | FAIL | `rc -1, first bad check -1 got 0x00000000 want 0x00000000` | unchanged |
| 241 es3_precision_int, 242 es3_precision_uint | REFUSED compile | fragment `an integer colour output other than one declared highp uvec4 (an ivec4, int or uint, fewer lanes, or a uvec4 mediump or highp only by precision highp int) is not lowered yet` | new case (was NO-CASE) |
| 245 mrt4 | REFUSED draw | `a colour clear of several targets other than one whole clear of every target before any draw is not built yet` | new case (was NO-CASE) |

SKIP, 8 cases: 97 varyings15, 200 intrt, 201 usamp, 222 dtpu, 223 msaarbo,
244 w128tex, 246 tsprobe, 247 dlayout. FAULT: none.

## 3. Per-case diff against 2026-11-t6-sweep-1a22bd38 (both runs)

By hand (`diff` of the two verdict files; `tools/cts/mali-sweep-tally.py`
is not in this tree yet). 28 cases differ; the other 228 verdict lines are
byte-identical.

| Case | 1a22bd38 | d9efa418 |
|---|---|---|
| 128 uniidxj | PASS | REFUSED compile |
| 149 vsunits | REFUSED draw | PASS |
| 177 zonly | SKIP | PASS |
| 178 heldfree | SKIP | PASS |
| 179 retile | SKIP | PASS |
| 206 es3levels | FAIL (check 2 got `0x04602020` want `0x04602090`) | PASS |
| 215 shadowmap | SKIP | PASS |
| 229 tf_deleted_output | FAIL | PASS |
| 230 flat_lines | NO-CASE | PASS |
| 231 flat_lines_wide | NO-CASE | PASS |
| 232 flat_line_strip | NO-CASE | PASS |
| 233 flat_line_strip_wide | NO-CASE | PASS |
| 234 flat_line_loop | NO-CASE | PASS |
| 235 flat_line_loop_wide | NO-CASE | PASS |
| 236 flat_tri_strip | NO-CASE | PASS |
| 237 flat_tri_fan | NO-CASE | PASS |
| 238 drawrandom_packed | NO-CASE | PASS |
| 239 drawrandom_intfloat | NO-CASE | PASS |
| 240 drawrandom_intw | NO-CASE | PASS |
| 241 es3_precision_int | NO-CASE | REFUSED compile |
| 242 es3_precision_uint | NO-CASE | REFUSED compile |
| 243 divmix | NO-CASE | PASS |
| 244 w128tex | NO-CASE | SKIP |
| 245 mrt4 | NO-CASE | REFUSED draw |
| 246 tsprobe | NO-CASE | SKIP |
| 247 dlayout | NO-CASE | SKIP |
| 248 tearbars | NO-CASE | PASS |
| 249 linewidth | NO-CASE | PASS |

(Cases 177 and 178 were named `?` in the 1a22bd38 lines; the names above
are this record's.) 21 cases join PASS, 1 leaves it (128).

## 4. Against the forecast

- 149, 206, 229, 177, 178, 179, 215 forecast to change: all seven did, each
  to PASS. As forecast.
- 230 to 249 forecast to be new cases: all twenty now run (NO-CASE 73 to 53,
  as forecast). Of them 14 PASS, 3 SKIP (244, 246, 247) and 3 are REFUSED
  (241, 242 compile, 245 draw); the earlier read-only session at this
  commit also saw 241, 242 and 245 as non-PASS.
- 128 forecast possibly to become REFUSED: it did (REFUSED compile, the
  fragment varying load or conversion refusal), leaving the PASS set, as
  the earlier session at this commit saw.
- 77, 106, 163, 226, 227 and 228: no change forecast, none seen (the same
  verdict lines byte for byte).
- No change outside the forecast.

## 5. Triage of the non-pass cases

Every REFUSED and FAIL line of `sweep-1.verdicts.txt` (the same in
`sweep-2.verdicts.txt`), traced from the source. FAULT: none. All file:line
references are at d9efa418 (`git show d9efa418:<path>`). The five compile
refusals were reproduced on the host at d9efa418. Each case's program went
through the online back end's GL-core entry points (`link_records`,
`src/shaderc/online/mali/tests.rs:9636`; features `host-test,gpu-mali,
shaderc-online,no-shader-table`), with every return of the named error
traced in a scratch copy. Each one refused with the board's text at the
line given below. The scratch copy was not committed. Only one raise site
of the named error was reached in each program.

The gl_test case code is `driver/tools/pvrlab/gl_test.c` and its
`gl_test_*.inc` files, built into `panthor-gltest`.
`tools/mali/panthor/src/bin/panthor-gltest.rs` only runs each case in a
child process and prints the verdict line.

| Case | Verdict | Cause | file:line at d9efa418 | Fix | Owning plan | Issue |
|---|---|---|---|---|---|---|
| 77 transc | REFUSED compile | The case's first program (`transc.frag`: sin, cos, log2, exp2 of uniform lanes) compiles. The second (`vtransc.frag`) refuses on `inversesqrt(vec2(1.0, u_v.w))`. The mediump fragment walk lowers `inversesqrt` only on a lane it computed itself or on a varying's lane. A uniform's lane has no reference. | `src/shaderc/online/mali/nir_fs.rs:7578` (`Frsq`/`Fsqrt` arm: `alu_of(x).is_none() && input_lane(x).is_none()` returns `MaliError::Arithmetic`; text at `src/shaderc/online/mali/mod.rs:424`) | Take a drm-shim reference of `vtransc.frag` (`FRSQ.f16` of a uniform lane read from the FAU, with the constant lane folded). Then accept a uniform lane and a constant lane in that arm when the reference's words match. | #402 / #387 (src/shaderc/online/mali/**) | #33 |
| 106 mrt3 | REFUSED draw | Part 1 clears the three targets to grey, then calls `glFinish`, so the clear goes to the GPU as a frame of its own. The draw after it opens a new frame of three targets that starts from their old images. The colour preload draws back target 0 only. | `src/mali/backend.rs:2968` (`open_frame`: `clear.is_none() && n > 0` returns `NOT_BUILT_TARGETS_PRELOAD`, text at `src/mali/backend.rs:210`; host test `src/mali/backend_tests.rs:1025` "two targets and no clear") | Give the colour preload one draw-back per colour target: one preload program output and Render Target a target, as Mesa's `pan_preload` does for each cbuf. Then drop the refusal. | #406 (src/mali/backend.rs); frame building also #402 (src/mali/frame.rs) | #33 |
| 128 uniidxj | REFUSED compile (PASS at 1a22bd38) | `gl_FragColor = vec4(v_color.rg, fcols[fidx].ba)`. The colour's lanes are the mediump varying's `rg` beside an indexed uniform's lanes, and only two of the four varying lanes are read. `colour_keeps_whole` answers only measured forms (the varying's lanes alone, or beside a constant, another varying or gl_PointCoord). A lane beside a uniform is "unsure", and since not every lane is read it refuses. | `src/shaderc/online/mali/nir_fs.rs:9183` (`colour_keeps_whole`: `unsure && !all_read` returns `MaliError::FsVaryingLoad`; the comment at `:9172-9173` names `uniidx_j`; text at `src/shaderc/online/mali/mod.rs:541`) | Take a drm-shim reference of `uniidx_j.frag` with `gl_test_passthrough_vs_src` (Mesa's slot width for `v_color`, and its one-word indexed uniform read where ours used `LD_PKA.i64`). Then add that form (varying lanes beside a uniform's lanes) to `colour_keeps_whole` with the reference as its test. | #402 / #387 (src/shaderc/online/mali/**) | new A |
| 163 drawbuf0 | REFUSED draw | `glDrawBuffers({GL_NONE, GL_COLOR_ATTACHMENT1})`: fragment output 0 goes nowhere and output 1 goes to target 0. The GL core passes this as `targets[0].out_shift != 0`, and the back end does not shift the outputs. The case's control draw and its checks pass; the case accepts the refusal. | `src/mali/backend.rs:1506` (`check`: `d.targets[0].out_shift != 0` returns `NOT_BUILT_OUTPUT_SHIFT`, text at `src/mali/backend.rs:217`; host test `src/mali/backend_tests.rs:1035`) | Build the shift: bind the blend descriptor and Render Target of output `out_shift` to target 0 (a blend shader or blend descriptor a slot, with the empty slot's write off), as Mesa maps cbufs past a NULL one. Then drop the refusal. | #406 (src/mali/backend.rs, src/mali/frame.rs; frame.rs also #402) | #33 |
| 226 negapi_uniformui | REFUSED compile | `o_color = vec4_v + vec4(ivec4_f) + vec4(uvec4_f) + texture(sampler_f, ..)` in a mediump shader. The int and uint uniforms are converted to float at fp32 (32-bit words). The fp16 `vec4_v` and the fp16 texel are added to them, so one sum mixes fp32 and fp16 operands. | `src/shaderc/online/mali/nir_fs.rs:6906` (the `Fmul`/`Fadd` arm: an operand neither fp32 nor fp16 nor a raw varying returns `MaliError::MixedPrecision`; text at `src/shaderc/online/mali/mod.rs:441`) | Lower Mesa's edge conversions: `F32_TO_F16` of the converted integer lanes (lower_precision narrows `i2f`/`u2f` results feeding a mediump tree), proven by a drm-shim reference of this shader. | #402 / #387 (src/shaderc/online/mali/**) | #4496 |
| 227 negapi_uniformuiv | REFUSED compile | The same shader (`negapi_uni_fs_src`, `driver/tools/pvrlab/gl_test_es3_negapi.inc:164`) as 226. The case differs only in its glUniform*uiv calls, which never run because the link refuses first. | `src/shaderc/online/mali/nir_fs.rs:6906`, as 226 | As 226. | #402 / #387 | #4496 |
| 228 negapi_readpix_fmt | FAIL (rc -1, no pixel check) | rc -1 is the case's `return ok ? 0 : -1` (`driver/tools/pvrlab/gl_test_es3_negapi.inc:340`). Its `na_expect` checks fail because the Mali back end has (1) no texture storage class, so the GL core refuses `glTexImage2D(GL_RGBA32I)` and `(GL_RGBA32UI)` by name with GL_INVALID_OPERATION (`na_expect(what, GL_NO_ERROR)` at `:296`, then an incomplete FBO), and (2) no samples (`max_samples` 0), so `glRenderbufferStorageMultisample(4, GL_RGBA8)` is GL_INVALID_OPERATION (`:314`) and `GL_SAMPLE_BUFFERS` reads 0. "first bad check -1 got 0x0 want 0x0" means no pixel was compared. | `src/gl/texture.rs:6526` (`es3_storage_or_refuse`: GL_INVALID_OPERATION when `storage_here()` is None) with `src/mali/backend.rs:116` (`SAMPLED_TEXFMTS: u32 = 0`); `src/gl/msaa.rs:68` (`requested > max.max(1)`: GL_INVALID_OPERATION), set at `src/gl/framebuffer.rs:3780`, with `src/mali/backend.rs:3819` (`max_samples: 0`) | In the Mali back end: list the RGBA32I/RGBA32UI classes in `SAMPLED_TEXFMTS` once the texture descriptor writes them, and set `max_samples` 4 once a 4x target is resolved (the frame already has a 4x path, `sample_store`). The GL core needs no change; `tests/es3_negapi_readpixels.rs` pins both rules on a stand-in back end. | #406 (src/mali/backend.rs) | #4496 |
| 241 es3_precision_int | REFUSED compile | `layout(location = 0) out highp int o_out` (then mediump and lowp): an integer colour output that is not a `highp uvec4`. Only `highp uvec4` has a Mesa reference (`BLEND.v4.u32`). All 9 rows refuse; the host check covered `+` at each precision. | `src/shaderc/online/mali/mod.rs:1189` (`ty.base != UVec(4) \|\| ty.precision != High` returns `MaliError::IntColour`; text at `src/shaderc/online/mali/mod.rs:532`) | Take drm-shim references of the scalar `int` and `uint` outputs (and `ivec4`, mediump, lowp) and lower their `BLEND` forms (`.i32`, fewer lanes). The draw then meets the Mali back end's integer target refusal (`INTEGER_RT_FORMATS` 0 at `src/mali/backend.rs:120`, `NOT_BUILT_TARGET_FORMAT` at `src/mali/backend.rs:1593`), which needs an RGBA32I/RGBA32UI Render Target class. | #402 / #387 (compiler); #406 (the integer target in src/mali/backend.rs) | new B |
| 242 es3_precision_uint | REFUSED compile | As 241 with `uint` (scalar, at each precision): still not a `highp uvec4`. | `src/shaderc/online/mali/mod.rs:1189`, as 241 | As 241 (the scalar `uint` reference and `BLEND.u32` of one lane, and the RGBA32UI target). | #402 / #387; #406 | new B |
| 245 mrt4 | REFUSED draw | Part 1 clears each of the four draw buffers with its own colour (`glClearBufferfv(GL_COLOR, n, ..)`, `driver/tools/pvrlab/gl_test_es3_mrt4.inc:153`). The first clear names draw buffer 0 alone (`clear_target_mask` 0x1). The back end builds a several-target clear only as one whole clear of every target, because the clear quad writes target 0 only. Part 4's scissored `glClearBufferfv(GL_COLOR, 3, ..)` (`:191`) is the same class. | `src/mali/backend.rs:1567` (`check_targets`: `d.do_clear && (clear_quad \|\| colour_clear_mask != 0xf \|\| !all)` returns `NOT_BUILT_TARGETS_CLEAR`, text at `src/mali/backend.rs:215`; the frame builder's second guard is `:3440`; host test `src/mali/backend_tests.rs:1028` "glClearBuffer of draw buffer 1 alone") | Clear per target: keep a clear colour per Render Target (the frame's clear values, one a target, for a whole clear at frame start), and for a clear after a draw or a scissored/masked one, a clear quad whose program writes one output a target with the other targets' writes off. Then drop both guards. | #406 (src/mali/backend.rs; frame.rs also #402) | new C |

### 128: the PASS regression

`git log -S 'uniidx_j' 1a22bd38..d9efa418 -- src/shaderc/online/mali` and a
host bisect of the same program (fragment refusal or not at each commit
from 1a22bd38 to d9efa418) name **f4332514**, "mali: a colour store keeps a
varying whole by the lanes it writes, each answer a measured form"
(2026-11-12). Its parent 22527163 compiles the program; f4332514 refuses it
with the board's text. The commit made `colour_keeps_whole` a whitelist of
measured forms, and its message says `es100/uniidx_j.frag` moves from ok to
FsVaryingLoad in `tools/mali/corpus-verdicts.txt`. At 1a22bd38 the program
compiled to words that are not Mesa's (its indexed uniform read as
`LD_PKA.i64` where Mesa reads one word). It drew the expected pixel, but
nothing showed it matched Mesa. The refusal is deliberate, not a code
generation regression: the case leaves PASS until the form has a
reference (issue A). babf5f2a, which `git log -S` on the refusal text
names (it reworded the message), still compiles this program (bisect).

### Grouping against the earlier triage

Confirmed as given: new A (128), new B (241, 242), new C (245); #33 keeps
77, 106 and 163; #4496 keeps 226, 227 and 228. Three corrections to that
triage. 228's rc -1 comes from the gl_test case code
(`driver/tools/pvrlab/gl_test_es3_negapi.inc:340`), not from
`tools/mali/panthor/src/`. 77's refusal is in its second program
(`vtransc`, `inversesqrt` of a uniform lane), not in sin/cos/log2/exp2.
The fix for 241/242 reaches src/mali/** too (integer targets).

### Issues

**A. `mali: a colour of mediump varying lanes beside an indexed uniform's lanes refuses (FsVaryingLoad)`**

    gl_test 128 uniidxj is REFUSED compile on the NanoPC-T6 at d9efa418
    (tools/cts/log/2026-11-26-t6-sweep-d9efa418); it was PASS at 1a22bd38.
    Refusal: "Valhall: a fragment varying load or conversion of a form no
    reference draws (...) is not lowered yet", raised at
    src/shaderc/online/mali/nir_fs.rs:9183 (colour_keeps_whole, unsure && !all_read).
    The colour vec4(v_color.rg, fcols[fidx].ba) puts a uniform's lanes beside two
    of four varying lanes, a form with no drm-shim reference; f4332514 made it refuse.
    Fix: a drm-shim reference of es100/uniidx_j.frag with the passthrough vertex
    shader (slot width, one-word indexed uniform read), then that form in
    colour_keeps_whole with the reference as its test. Plans #402 / #387.

**B. `mali: integer colour outputs other than highp uvec4 refuse (IntColour)`**

    gl_test 241 es3_precision_int and 242 es3_precision_uint are REFUSED compile on
    the NanoPC-T6 at d9efa418 (tools/cts/log/2026-11-26-t6-sweep-d9efa418).
    Refusal: "Valhall: an integer colour output other than one declared `highp uvec4`
    (...) is not lowered yet", raised at src/shaderc/online/mali/mod.rs:1189.
    The cases write a scalar highp/mediump/lowp int or uint to an RGBA32I/RGBA32UI target.
    Fix: drm-shim references of scalar int/uint (and ivec4, mediump, lowp) outputs
    and their BLEND forms in the compiler (plans #402 / #387); then an RGBA32I/RGBA32UI
    Render Target class in the back end, which refuses it today
    (INTEGER_RT_FORMATS 0, src/mali/backend.rs:120; NOT_BUILT_TARGET_FORMAT, :1593; plan #406).

**C. `mali: a colour clear of some of several targets (glClearBuffer per buffer, scissored) refuses`**

    gl_test 245 mrt4 is REFUSED draw on the NanoPC-T6 at d9efa418
    (tools/cts/log/2026-11-26-t6-sweep-d9efa418).
    Refusal: "mali: a colour clear of several targets other than one whole clear of
    every target before any draw is not built yet", raised at src/mali/backend.rs:1567
    (check_targets), with a second guard at :3440 (the clear quad writes target 0 only).
    The case clears each of four draw buffers with its own glClearBufferfv, then a
    scissored clear of buffer 3.
    Fix: a clear colour per Render Target for a whole clear at frame start, and a
    clear quad writing one output per target (others' writes off) for the rest;
    then drop both guards. Plan #406 (src/mali/backend.rs, src/mali/frame.rs).

## 6. Replacement texts for the owners

Paste-ready, for the owners named in (f). Figures are those of sections 1 to
5: 256 cases, PASS 185, REFUSED 9, NO-CASE 53, SKIP 8, FAIL 1, FAULT 0, 0
kernel messages. Issue letters A, B and C are the new issues of section 5's
"Issues"; 77, 106 and 163 stay on #33, 226, 227 and 228 on #4496. Each text
keeps the letters until the issues are filed; then replace each letter with
its number. Nothing here says that an old issue (#4350, #32, #3889) is
closed: its owner decides.

### (a) tools/cts/RESULTS.md, section "Mali: NanoPC-T6 (RK3588, Valhall v10, panthor), gl_test sweep"

Replaces the text from the heading's first paragraph to the bullet on cases
223 to 229 (the glmark2-es2 subsection after it stays).

    `panthor-gltest --sweep` on the board's own Linux (panthor, not Zephyr), the
    newest record: `tools/cts/log/2026-11-26-t6-sweep-d9efa418/` (2026-11-26,
    the board up since 2026-11-24). Code: origin/main `d9efa418`, cross-built
    and run by `tools/mali/panthor/run-on-board.sh` in a clean tree
    (`yakogl-build=d9efa418`). Kernel 6.18.55-current-rockchip64, panthor 1.5.0.
    Two sweeps in one boot; boot_id unchanged; `/boot` (419 files) and loaded
    modules (100) unchanged. The previous record is
    `tools/cts/log/2026-11-t6-sweep-1a22bd38/` (main `1a22bd38e`, 2026-11-12).

    | Runs | Cases | PASS | REFUSED | NO-CASE | SKIP | FAIL | FAULT | Kernel messages |
    |---|---:|---:|---:|---:|---:|---:|---:|---:|
    | sweep 1 and 2, equal (the 256 verdict lines are identical) | 256 | **185** | 9 | 53 | 8 | 1 | 0 | 0 |

    - Against 1a22bd38 (165 PASS, 6 REFUSED, 73 NO-CASE, 9 SKIP, 3 FAIL): 28
      cases differ, the other 228 verdict lines are byte-identical. 21 cases
      join PASS, 1 leaves it. Seven old cases moved as forecast, each to PASS:
      149 vsunits (was REFUSED draw), 206 es3levels (was FAIL), 229
      tf_deleted_output (was FAIL), 177 zonly, 178 heldfree, 179 retile and
      215 shadowmap (were SKIP). Cases 230 to 249 are new in gl_test (NO-CASE
      73 to 53): 14 PASS, 3 SKIP (244 w128tex, 246 tsprobe, 247 dlayout) and 3
      REFUSED (241, 242, 245).
    - PASS regression, 1: 128 uniidxj, PASS at 1a22bd38, REFUSED compile now.
      It is a deliberate refusal, not a code generation fault: f4332514 made
      the fragment colour store accept only measured forms, and a colour of
      two mediump varying lanes beside an indexed uniform's lanes has no
      Mesa reference (A).
    - FAIL, 1 line: 228 negapi_readpix_fmt (`rc -1, first bad check -1`:
      the Mali back end has no integer texture class and no samples, so
      `glTexImage2D(GL_RGBA32I)` and a 4x renderbuffer give GL error 0x502,
      #4496).
    - REFUSED, 9 lines: 77 transc (compile: `inversesqrt` of a uniform lane
      in the second program, #33), 106 mrt3 (draw: several colour targets in
      a frame that does not start from a whole clear, #33), 163 drawbuf0
      (draw: first draw buffer GL_NONE, #33), 226 and 227 (compile: a fragment
      shader that mixes highp and mediump float values, #4496), 128 uniidxj
      (compile: fragment varying load or conversion, A), 241 and 242
      (compile: an integer colour output other than a highp uvec4, B), 245
      mrt4 (draw: a colour clear of some of several targets, C).
    - SKIP, 8: 97, 200, 201, 222, 223, 244, 246, 247. FAULT: none. Kernel
      messages: 0 between the run markers and 0 in the second after the end
      marker, in both runs.
    - Triage of every REFUSED and FAIL line (cause, file:line, fix, owning
      plan) is in the record's README, section 5.

### (b) tools/cts/RESULTS.md, the row cell

The row of the table in (a) (the NanoPC-T6 (Mali) row of the glmark2 table
points at this section and needs no change):

    | sweep 1 and 2, equal (the 256 verdict lines are identical) | 256 | **185** | 9 | 53 | 8 | 1 | 0 | 0 |

### (c) ROADMAP.md, stage 5, the sweep bullet

    - `gl_test` sweep (`tools/cts/log/2026-11-26-t6-sweep-d9efa418/`, main
      `d9efa418`, 2026-11-26): 185 of 256 cases PASS twice, 9 refused, 53 with
      no case, 8 skipped, 1 FAIL, 0 FAULT, 0 kernel messages. Since
      1a22bd38 (165 PASS): +21 PASS (149, 206, 229, 177, 178, 179, 215 and
      14 of the new cases 230 to 249), -1 (128 uniidxj, a deliberate refusal
      until the form has a Mesa reference). The 10 cases that are not PASS,
      NO-CASE or SKIP are triaged in the record: 77, 106, 163 (#33), 226,
      227, 228 (#4496), 128 (A), 241 and 242 (B), 245 (C).

### (d) tools/cts/STATUS.md, the Japanese T6 line

Replaces the bullet that begins "NanoPC-T6（RK3588、Mali Valhall v10、panthor）の gl_test 掃引。".

    - **NanoPC-T6（RK3588、Mali Valhall v10、panthor）の gl_test 掃引。** 最新の記録は tools/cts/log/2026-11-26-t6-sweep-d9efa418（main d9efa418 を tools/mali/panthor/run-on-board.sh でクロスビルドして実行、2026-11-26）。panthor-gltest --sweep は 1 回の起動のうちに 2 回走り、256 件中 185 合格（REFUSED 9、NO-CASE 53、SKIP 8、FAIL 1、FAULT 0）、2 回の判定行は同一、カーネルメッセージは 0 件、boot_id は不変。前の記録（2026-11-t6-sweep-1a22bd38、165 合格）から 28 件が変わった: 21 件が合格に加わり（149 vsunits、206 es3levels、229 tf_deleted_output、177 zonly、178 heldfree、179 retile、215 shadowmap と、新しいケース 230〜249 のうち 14 件）、1 件が合格から外れた（128 uniidxj。fragment の色が mediump の varying のレーンと添字付き uniform のレーンを並べる形に Mesa の参照が無く、意図して拒否している。issue A）。合格でないもの: REFUSED は 77（vtransc の inversesqrt が uniform のレーンを取る、#33）、106（全面クリアで始まらない複数カラーターゲット、#33）、163（先頭の draw buffer が GL_NONE、#33）、226 と 227（highp と mediump が混ざる fragment shader、#4496）、128（issue A）、241 と 242（highp uvec4 以外の整数カラー出力、issue B）、245（複数ターゲットの一部だけのカラークリア、issue C）。FAIL は 228（整数テクスチャとマルチサンプル renderbuffer が未実装、#4496）。詳細は RESULTS.md と記録の README。

### (e) The comment for issue #7

    NanoPC-T6, gl_test sweep at origin/main d9efa418 (2026-11-26), on the
    board's own Armbian through panthor, twice in one boot:
    tools/cts/log/2026-11-26-t6-sweep-d9efa418/.

    256 cases: PASS 185, REFUSED 9, NO-CASE 53, SKIP 8, FAIL 1, FAULT 0. The
    two runs' verdict lines are identical, 0 kernel messages, boot_id
    unchanged. Against the record at 1a22bd38 (PASS 165, REFUSED 6, NO-CASE
    73, SKIP 9, FAIL 3): 28 cases changed, +21 PASS (149, 206, 229, 177,
    178, 179, 215 and 14 of the 20 new cases 230 to 249), -1 PASS (128
    uniidxj, a deliberate refusal). Every change matches the forecast.

    Not PASS, each traced to a file and line in the record's README:
    - 77, 106, 163: still #33 (compiler and back end).
    - 226, 227, 228: still #4496 (226 and 227 the highp/mediump mix, 228 the
      missing integer texture class and samples).
    - 128: new issue A (fragment varying lanes beside an indexed uniform).
    - 241, 242: new issue B (integer colour outputs other than highp uvec4;
      also needs an integer Render Target class in the back end).
    - 245: new issue C (a colour clear of some of several targets).
    Stage 5's criterion is not met yet (1 FAIL, 9 REFUSED).

### (f) Who takes each text

| Text | Goes to | Owner |
|---|---|---|
| (a), (b): the RESULTS.md T6 gl_test sweep section and its row | tools/cts/RESULTS.md | plan #387 |
| (c): the stage 5 sweep bullet | ROADMAP.md | plan #389 |
| (d): the Japanese T6 line | tools/cts/STATUS.md | plan #392 (README Status; STATUS.md follows it) |
| (e): the #7 comment | issue #7 | whoever files A, B and C with it |
| Issues A, B, C (section 5, "Issues") | new issues | whoever files them |
