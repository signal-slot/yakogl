<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The ST row's line width (P9, Q11)

The STM32MP257F-DK's GPU is the GCNANOULTRA31_VIP2 row (`src/vivante/profile.rs`,
the ST table). This note states what line width that row supports, what our GL
core reports and programs today, which cases depend on it, and the decision for
P9 (`docs/notes/pre-study-stm32mp257f-dk.md`, Q11 at :876 and the P9 row at
:965). Nothing here was measured on the board.

Sources: Mesa's `etnaviv_screen.c:366-368` and the hwdb lines are **cited**
from `src/vivante/profile.rs` and `tools/vivante/README.md`, not re-read for
this note: no `$MESA_ETNA_TREE` was set and the fetch of Mesa 26.2.1 into
`$CONNECTED_DEPS_DIR` was refused in this session. The dEQP sources were not
opened either; what dEQP requires of the range is stated as unread below.

## 1. What the row supports

- The ST row has the feature bit `WIDELINE_TRIANGLE_EMU` set
  (`src/vivante/profile.rs:152`, hwdb `:5182`); the i.MX8MP's GC7000UL has it
  clear (hwdb `:19297`).
- The bit means the driver draws wide lines as triangles, so Mesa reports a
  line width of 1.0 at most (`src/vivante/profile.rs:61-63`, citing
  `etnaviv_screen.c:366-368`: 1.0 with the bit, 8192.0 without).
- Mesa on a drm-shim run reports 1..1 on the ST row and 1..8192 on the
  i.MX8MP; for `glLineWidth(4)` `PA_LINE_WIDTH` and `PA_WIDE_LINE_WIDTH0/1`
  hold 0.5 on the ST row (clamped to 1.0, registers hold half the width) and
  2.0 on the i.MX8MP (`tools/vivante/README.md:744-751`).
- The profile's range is `[1.0, 1.0]` "until a board test says what the
  wide-line registers do on this part" (`src/vivante/profile.rs:163-165`); the
  i.MX8MP row is `[1.0, 16.0]` (`:128-130`).

What the hardware does with a wide-line register value above 0.5 is the one
open question (`tools/vivante/README.md:757`, pre-study Q11).

## 2. What our GL core reports and programs today

- `GL_ALIASED_LINE_WIDTH_RANGE` is the constant 1.0 .. 16.0 for every build
  (`src/gl/state.rs:1038-1042`). The profile's `line_width_range` is not read
  by the GL state, so on the DK the core would report 1..16 (the i.MX8MP's
  value), not the ST row's 1..1.
- `glLineWidth` stores the width unclamped; only `width <= 0` is
  `GL_INVALID_VALUE` (`src/gl/state.rs:782-790`). The comment "the ISP takes
  up to 16 px" at `:790` is the PowerVR's, not Vivante's.
- The Vivante back end writes `width * 0.5` to `GC_PA_LINE_WIDTH`
  (`src/vivante/backend.rs:1608`) and to `GC_PA_WIDE_LINE_WIDTH0/1`
  (`:1676-1677`), unclamped. On the ST row `glLineWidth(6)` programs 3.0
  where Mesa's golden stream programs 0.5. The pre-study's line numbers for
  these writes (`:426`, `:876`: `backend.rs:1376`, `:1425-1426`) have drifted
  to the ones above.

## 3. What depends on it

dEQP-GLES2 (the DK's sweep list leaves these out as "pending Q11",
`tools/cts/mp2-deqp-gles2-exclude.txt:19-39`; the must-pass list has them at
`tools/cts/gles2-mustpass.txt:122-129`, `:3576`, `:3944-3967`,
`:14172-14174`):

- `dEQP-GLES2.functional.clipping.line.*wide*`: 8 cases.
- `dEQP-GLES2.functional.rasterization.interpolation.{basic,projected}.
  {line_loop,line_strip,lines}_wide` and
  `rasterization.primitives.{line_loop,line_strip,lines}_wide`: 9 cases.
  These need real wide lines.
- `dEQP-GLES2.functional.implementation_limits.aliased_line_width_range` and
  `state_query.floats.aliased_line_width_range_{getboolean,getfloat,getinteger}`:
  they read the *report*, not the drawing. Whether dEQP requires a minimum
  maximum width for GLES2 is unread, so they are expected to pass with 1..1
  only if it does not.
- `negative_api.fragment.line_width` (`gles2-mustpass.txt:3687`) tests
  `glLineWidth(0)`/negative values: independent of the range.
- Whether the wide cases shrink their width to the reported maximum or fail
  at a maximum of 1 is unread.

`gl_test` (`driver/tools/pvrlab/gl_test.c`):

- Case 10 `prims`: `glLineWidth(3.0f)` on a LINE_LOOP, then the check
  "loop: 3 px wide" at pixel (17, 32) (`gl_test.c:4331`, `:4344`). It needs a
  real 3-pixel line.
- Cases 231, 233, 235 (`flat_lines_wide`, `flat_line_strip_wide`,
  `flat_line_loop_wide`; names at `gl_test.c:487-489`): they ask for width 5
  clamped to the reported range; with a maximum of 1 "the case then runs at
  width 1 and says so" (`driver/tools/pvrlab/gl_test_es3_flat.inc:43-46`,
  cases at `:212-228`).
- `panel_demo` draws a 6 px frame (`gl_test.c:24355`): a visual demo, no check.
- Every other case runs at width 1 (`gl_test.c:1508`, `:13819`).

## 4. The options

1. **Report the profile's range (1..1) and clamp what is programmed to it.**
   The GL state reads `profile::ACTIVE.line_width_range`, the back end writes
   `min(width, range max) * 0.5`. This is what Mesa does on this part, so our
   stream matches the golden one (0.5 in the three registers).
2. **Report 1..16 and program the wide-line registers** (today). Unverified on
   the hardware; the registers hold values Mesa never writes on this part,
   and a case that asks for the reported maximum hits that untested path.
3. **Emulate wide lines as triangles.** What the feature bit says the driver
   would do: a new expand-to-quad geometry path with its own clipping,
   interpolation and flat shading, beside the i.MX8MP's native one. That is a
   feature, not a Q11 answer.

## 5. Decision

**Report the ST profile's range, 1.0 to 1.0, and clamp** (option 1). Reason:
the DK's stream then equals Mesa's golden stream for the part
(`tools/vivante/README.md:748-751`) and no untested wide-line register value
reaches the silicon. The i.MX8MP stays at 1..16 and unchanged because its
profile says so (`src/vivante/profile.rs:130`).

The cost, stated: the 17 dEQP-GLES2 wide-line cases and `gl_test` case 10's
"3 px wide" check stay expected differences on the DK (the exclude file's
"pending Q11" reason becomes "wide lines are not supported by the part's
reported range"); the four range-query cases move back into the sweep;
cases 231/233/235 pass as width-1 cases by design.

Emulation (option 3) is out of P9. It is the follow-up only if a Qt or
glmark2 workload needs wide lines on the DK. The board test of what the
wide-line registers do is optional for this decision, because option 1 never
relies on its answer.

The code change (the GL state and back end reading `profile::ACTIVE`, the
exclude file's reason text) is a separate item, not part of this note.
