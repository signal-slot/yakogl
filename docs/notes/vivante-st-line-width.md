# The ST row's line width (P9, Q11)

The STM32MP257F-DK's GPU is the GCNANOULTRA31_VIP2 row (`GCNANOULTRA31_VIP2_ST`,
`src/vivante/profile.rs:138`). This note states what line width that row
supports, what our GL core reports and programs today, which cases depend on
it, and the decision for P9 (`docs/notes/pre-study-stm32mp257f-dk.md`, Q11 and
the P9 row). Nothing here was measured on the board.

`[unread]` marks a source this note could not open. Mesa's tree was not
available: `$MESA_ETNA_TREE` is unset here, no etnaviv source is under
`$CONNECTED_DEPS_DIR`, and fetching it needs network approval this run did not
have. Every Mesa claim below is therefore second-hand, from the repository's
own record of it, and carries `[unread]`. Whoever has a Mesa 26.2.1 tree reads
`src/gallium/drivers/etnaviv/etnaviv_screen.c:366-368` and the etnaviv hwdb row
and confirms the three cited lines.

## 1. What the row supports

| Fact | Value | Source |
| --- | --- | --- |
| Feature bit `WIDELINE_TRIANGLE_EMU` | set on the ST row, clear on the i.MX8MP's GC7000UL | `src/vivante/profile.rs:152` (ST), `:117` (i.MX8MP); the hwdb lines `:5182` (ST) and `:19297` (i.MX8MP) are quoted there, [unread] |
| What the bit means | the driver draws wide lines as triangles; Mesa then reports a line width of 1.0 at most | `src/vivante/profile.rs:61-64`, citing `etnaviv_screen.c:366-368` [unread] |
| Mesa's range on the i.MX8MP / the ST row | 1..8192 / 1..1 | `tools/vivante/README.md:737` (a Mesa 26.2.1 run on drm-shim, per that section; the Mesa source itself [unread]) |
| What Mesa programs for `glLineWidth(4)` | `PA_LINE_WIDTH` and `PA_WIDE_LINE_WIDTH0/1` hold 0.5 on the ST row (clamped to 1.0, registers hold half the width), 2.0 on the i.MX8MP | `tools/vivante/README.md:739-743` |
| The profile's range | `[1.0, 1.0]`, "until a board test says what the wide-line registers do" | `src/vivante/profile.rs:163-165`; the i.MX8MP row is `[1.0, 16.0]` (`:128-130`) |

The facts above say what Mesa does and what the hwdb flags. They do not say
what the hardware does with a wide-line register value above 0.5: the flag
only says the *driver* is expected to emulate wide lines, and Mesa chooses not
to. That is the one open hardware question (`tools/vivante/README.md:744-745`,
pre-study Q11 `:876`).

## 2. What our GL core reports and programs today

- `glGetFloatv(GL_ALIASED_LINE_WIDTH_RANGE)` writes the constants 1.0 and 16.0
  for every build (`src/gl/state.rs:1038-1042`); a host test pins 1..16
  (`src/gl/state.rs:2629`) and only checks that the range includes 1.0
  (`:2613-2615`). `profile::line_width_range` is read nowhere outside
  `src/vivante/profile.rs` (grep `line_width_range src`: `profile.rs:63, 97,
  130, 165` only). So the ST profile's `[1.0, 1.0]` does not reach the GL
  state: on the DK the core would report 1..16, which is the i.MX8MP's value
  and not the ST row's.
- `glLineWidth` stores the width unclamped; only `width <= 0` is
  `GL_INVALID_VALUE` (`src/gl/state.rs:782-790`; the comment there, "the ISP
  takes up to 16 px", is the PowerVR's and does not hold for Vivante).
  `GL_LINE_WIDTH` returns what was set (`:1022-1023`), as Mesa does
  (`tools/vivante/README.md:738`).
- The Vivante back end writes `width * 0.5` to `GC_PA_LINE_WIDTH`
  (`src/vivante/backend.rs:1608`), sets `GC_PA_CONFIG_WIDE_LINE` for every draw
  (`:1627`) and writes `width * 0.5` to `GC_PA_WIDE_LINE_WIDTH0/1`
  (`:1676-1677`). No clamp. On the ST row a `glLineWidth(6)` therefore programs
  3.0 where Mesa's golden programs 0.5; the hardware's response is unknown
  (see section 1). The pre-study's `:1376` and `:1425-1426` line numbers for
  these writes (`pre-study-stm32mp257f-dk.md:426`, `:876`) have drifted: the
  lines are now the ones above.

## 3. What depends on it

dEQP-GLES2 (the DK's sweep list leaves these out as "pending Q11",
`tools/cts/mp2-deqp-gles2-exclude.txt:12-40`; the must-pass list has them at
`tools/cts/gles2-mustpass.txt:122-129`, `:3576`, `:3943` and following):

- `dEQP-GLES2.functional.clipping.line.*wide*`, 8 cases
  (`mp2-deqp-gles2-exclude.txt:19-26`).
- `dEQP-GLES2.functional.rasterization.interpolation.{basic,projected}.
  {line_loop,line_strip,lines}_wide` and
  `rasterization.primitives.{line_loop,line_strip,lines}_wide`, 9 cases
  (`:27-35`).
- `dEQP-GLES2.functional.implementation_limits.aliased_line_width_range` and
  `state_query.floats.aliased_line_width_range_{getboolean,getfloat,getinteger}`
  (`:36-39`). These read the reported range: they pass with any range that
  includes 1 and agrees between the getters, so they depend on the *report*,
  not on drawing. Whether dEQP also requires a minimum maximum for GLES2 is
  [unread] (dEQP sources not opened).
- `dEQP-GLES2.functional.negative_api.fragment.line_width`
  (`tools/cts/gles2-mustpass.txt:3687`) tests `glLineWidth(0)` and negative
  values: independent of the range.
- Whether dEQP's wide-line cases shrink their width to the reported maximum or
  fail when the maximum is 1 is [unread].

`gl_test` (`driver/tools/pvrlab/gl_test.c`):

- Case 10 `prims`, frame 2: `glLineWidth(3.0)` on a LINE_LOOP, then checks
  pixel (17, 32) is green ("loop: 3 px wide",
  `gl_test.c:4331-4344`). This case needs an actual 3-pixel line; on an
  emulating core with range 1..1 it is expected to differ. Case 1 (the DK's
  first case) draws no line (`docs/notes/stm32mp257f-dk-bringup.md:270-271`).
- Cases 231, 233, 235 (`flat_lines_wide`, `flat_line_strip_wide`,
  `flat_line_loop_wide`): they ask for width 5 clamped to the *reported* range,
  and when the maximum is 1 "the case then runs at width 1 and says so"
  (`gl_test_es3_flat.inc:43-46`, `:140-151`, `:212-213`). With the report fixed
  at 1..1 they pass as width-1 cases; with today's 1..16 report they ask for 5
  and run the wide path. Their dEQP-GLES3 flat-shading counterparts are named in
  `gl_test.c:487`.
- `panel_demo` draws a 6 px frame (`gl_test.c:24355`); a visual demo, no check
  (not a regression case).
- Case 127 `lines` and case 56 `manylines` draw width-1 lines
  (`gl_test.c:1508` resets width to 1 for every case); width 1 is the same on
  both rows.

## 4. The options

1. **Report the profile's range (1..1) and clamp what is programmed to it.**
   The GL state reads `profile::ACTIVE.line_width_range` instead of the
   constant, and the back end writes `min(width, range max) * 0.5`. This is
   exactly what Mesa does on this part (sections 1 and 3 of
   `tools/vivante/README.md`'s Q11), so our stream matches the golden one
   (0.5 in the three registers) and the ST row's expected differences from the
   i.MX8MP stay inside what the hwdb diff says.
2. **Report 1..16 and program the wide-line registers** (today). Unverified on
   the hardware, the registers hold values Mesa's golden never writes on this
   part, and a conformance case that asks for the reported maximum would hit
   the untested path.
3. **Emulate wide lines as triangles.** What the feature bit says the *driver*
   would do. It means a new geometry path (an expand-to-quad step in the
   vertex stage or on the CPU), correct clipping, interpolation and
   flat-shading for the 9 + 8 dEQP cases, and a second code path for the
   i.MX8MP's native one. That is a feature, not a Q11 answer.

## 5. Decision

Report the ST profile's range, 1.0 to 1.0, and clamp: for the ST row P9 takes
option 1. `GL_ALIASED_LINE_WIDTH_RANGE` is read from the active profile
(`src/vivante/profile.rs:165`) rather than the constant at `src/gl/state.rs:1038-1042`,
and the back end programs `min(width, range max) * 0.5` at
`src/vivante/backend.rs:1608` and `:1676-1677`, so the DK's stream equals
Mesa's golden stream for the part (`tools/vivante/README.md:739-740`) and no
untested wide-line register value reaches the silicon; the i.MX8MP stays at
1..16 and unchanged because its profile says so (`profile.rs:130`). The cost
is explicit: the 17 dEQP-GLES2 wide-line cases and `gl_test` case 10's
"3 px wide" check stay listed as expected differences (the exclude file's
"pending Q11" reason becomes "wide lines are not supported by the part's
reported range", the four range queries move back into the sweep and are
expected to pass), while cases 231/233/235 pass as width-1 cases by design.
Emulation (option 3) is out of P9; it is the right follow-up only if a Qt or
glmark2 workload needs wide lines on the DK, and the board test of what the
wide-line registers do (Q11's open half) is optional for this decision
because option 1 never relies on the answer. The code change belongs to
`st-profile-plumb` (the GL state and back end reading `profile::ACTIVE`), not
to this note.
