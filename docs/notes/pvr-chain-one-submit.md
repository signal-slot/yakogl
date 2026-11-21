<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: one KCCB command for several renders (candidate 4 of am62p-slow-terrain.md), the experiment

Plan #329 item pvr-chain-one-submit-study, issue #1199. Written 2026-11-21
from the integration tip `975a6792`. Checked against origin/main `2be14a44`:
none of the files cited here differs between the two, so the line numbers
hold for both. This is a study. No code changed and no board was run. The
experiment below is described here and has not been landed.

It answers candidate 4 of `am62p-slow-terrain.md` ("Fix candidates, ranked",
`:288-302`): is the ~0.9 ms a queued kick costs the firmware's take-up of
each KCCB command, or of each render? If it is per command, posting a mipmap
chain's renders behind one command spreads the cost over the chain.

**Answer.**

- **(a) Yes, the firmware interface already lets one KCCB kick carry more
  than one render.** A KICK names no render. It carries a render context and
  a new client-CCB write offset for each data master (`ui32CWoffUpdate`).
  The firmware then runs every command in the client CCB up to that offset.
  N renders written back to back into the geometry and fragment client CCBs
  go out with one `COMBINED_TA_3D_KICK` whose two offsets point past the
  last one. No new field is needed. The limits are one render context a
  kick, and a cleanup-control list sized for one RT dataset.
- **(b) is therefore not needed as a fallback.** The closest form is the
  form itself: hold back the KCCB post of renders 1..N-1 and post render N.
  A bare doorbell (`wait_rering`) cannot stand in for it, because only a
  KICK carries the write offset.
- **The smallest experiment** is a few lines in the driver, a hold counter
  read once per submit with no struct layout change, plus three new arms in
  gl_test case 219 (`kickpair`): `two-held`, `four` and `four-held`, at
  64x64 and at 400x300. No new case number is taken. It needs one image and
  one board session. The decision rests on `(four - four-held) / 3` against
  `(four - one) / 3` in ms a rep.
- **Case 219 as landed cannot decide it.** In both its arms every render is
  a KCCB command of its own.

## (a) What the firmware interface allows

Everything below is from the vendored DDK headers
(`third_party/ti-img-rogue-ddk-25.2/include/rogue/`, read only).

| fact | where |
|---|---|
| A KCCB packet is a type, a flags word and a union of command datas, one of which is `sCombinedTA3DCmdKickData` | `rgx_fwif_km.h:1022-1035` (`ui32KCCBFlags` `:1025`) |
| `RGXFWIF_KCCB_CMD_KICK` is "DM workload kick", and `RGXFWIF_KCCB_CMD_COMBINED_TA_3D_KICK` is "There is a TA and a 3D command in this single kick" | `rgx_fwif_km.h:906-908`, `:938-940` |
| The kick data is `psContext` (the firmware context), `ui32CWoffUpdate` ("Client CCB woff update"), `ui32CWrapMaskUpdate`, `ui32NumCleanupCtl` and `apsCleanupCtl[]`. With `SUPPORT_WORKLOAD_ESTIMATION` it also has `ui32WorkEstCmdHeaderOffset`. There is no render count, no render list and no HWRT field | `rgx_fwif_km.h:581-598` |
| The combined kick is two of those: `sTACmdKickData` and `s3DCmdKickData` | `rgx_fwif_km.h:600-608` |
| `apsCleanupCtl` has `RGXFWIF_PRBUFFER_MAXSUPPORTED + 1` = 3 entries. The comment reads "+1 is RTDATASET cleanup", so the DDK's own use is one RT dataset a kick | `rgx_fwif_km.h:578-579`, `rgx_fwif_shared.h:125` |
| A cleanup control is two counts, `ui32SubmittedCommands` and `ui32ExecutedCommands`, kept for the HWRT data and ZS buffer lifetimes | `rgx_fwif_shared.h:111-120` |
| The client CCB is a queue of commands, not one command. The CCCB control comment draws Roff, Doff and Woff with several commands between them: "Cmd 1: currently executing ... Cmd 2,3,4: fence dependencies met, commands runnable ... Cmd 5...: fence dependency not met yet" | `rgx_fwif_shared.h:180-192`; the control words `:194-219` |
| Each command carries its own RT data: `CMDTA3D_SHARED.sHWRTData`, "RTData associated with this command, this is used for context selection". Renders on different HWRT members in one CCB walk are therefore the format's ordinary case | `rgx_fwif_shared.h:169-178` |
| Client CCB command kinds: `GEOM`, `3D`, `NULL`, `FENCE`, `UPDATE`, `PADDING`, each behind an `RGXFWIF_CCB_CMD_HEADER` {type, size, ext/int job ref} | `rgx_fwif_km.h:1316-1318`, `:1322-1324`, `:1341-1342`, `:1347-1351`, `:1368-1370`; the header `:1391-1408` |
| `RGXFWIF_KCCB_CMD_FORCE_UPDATE` ("Forcing signalling of all unmet UFOs for a given CCB offset", `ui32CCBFenceOffset`) is a recovery command, not a submit | `rgx_fwif_km.h:610-617`, `:935-937` |

How the driver uses this today:

- The host never writes a client CCB's write offset. `ctx_init` writes only
  the wrap mask of the two CCCB controls
  (`driver/rust/src/gpu/pvr/render.rs:572-575`). The firmware learns the
  offset only from a KICK's `ui32CWoffUpdate`. Commands written but not yet
  covered by a kick are invisible to it.
- `submit` (`render.rs:1389`) writes one render's GEOM + UPDATE (geometry
  UFO slot = seq) into the geometry CCB (`:1461-1468`), and FENCE (that
  slot) + 3D + UPDATE (fragment UFO = seq) into the fragment CCB
  (`:1560-1594`). It then builds the kick data from the running offsets
  `r.gwo` and `r.fwo` (`:1609-1621`) and posts one combined kick
  (`:1630-1637`) or two KICKs (`:1638-1644`). The offsets are cumulative,
  so a kick built after two renders' writes covers both.
- The driver already depends on one kick covering more than one render's
  commands. The 2026-09-24 mitigation says "the firmware sits on a posted
  kick until another command arrives" (`driver/src/pvr_render.c:354-368`).
  The command that arrives next is the next render's kick or the 20 ms
  re-post of the same bytes (`render.rs:2378-2400`), and the stalled
  render's commands then run under it. The re-post re-sends `r.last_kick`,
  the kick built at that render's submit (`render.rs:1635`, `:1639-1641`).

What one kick for N renders must respect:

1. **One render context.** `psContext` names the TA or 3D context of one
   `FWRENDERCONTEXT` (`render.rs:1619-1621`). The driver has one render
   context, so this holds.
2. **Cleanup controls.** Today's kick names the HWRT data of its own render
   (`render.rs:1608`, `:1615-1616`, `ui32NumCleanupCtl` = 1). A kick that
   covers renders on several HWRT members names only the last one.
   Whether the firmware counts `ui32SubmittedCommands` per listed control or
   per command walked is not in the headers. The counts matter only to a
   CLEANUP of that HWRT, which the driver sends only with `hwrt_cleanup`
   (`driver/src/pvr_render.c:485`, off by default). The experiment keeps it
   off. A product version would list every member the kick covers (up to 3
   fit) or keep the knob off.
3. **Ordering inside the walk.** Each 3D waits on its own render's geometry
   UFO slot (`render.rs:1588`). The firmware's UFO test is an equality test,
   which is why the slots are a ring of 16 (`render.rs:546-555`, `:54`,
   `geom_ufo_slot` `:108-110`). Several renders in one walk keep that. The
   3D data master runs one job at a time, in CCB order. Today's chain
   already relies on that between levels (`pvr-tq-mipmap.md`, "One submit
   carrying N level blits").
4. **Room.** `ring_reserve` pads at the wrap and never checks the firmware's
   read offset (`render.rs:1109-1122`). Its bound is the geometry UFO ring
   throttle, 16 kicks in flight (`:1401-1408`). One render is 112 + 400
   bytes (`GEOM_BYTES`/`FRAG_BYTES`, `render.rs:3949-3950`), against
   `CCCB_SIZE` = 32 KiB (`:48`). Nine held renders use 3.6 KiB of the
   fragment ring.

## (b) The other forms, and why they are not the experiment

- **Doorbell only** (`kccb_doorbell`, `driver/rust/src/gpu/pvr/ccb.rs:59-64`;
  `wait_rering`, `render.rs:2373-2377`). It carries no write offset, so it
  cannot publish held commands. It only asks the firmware to look at the
  KCCB again.
- **N KICKs written to N KCCB slots with one write-offset update and one
  doorbell.** Today `kccb_post` (`ccb.rs:68-112`) writes the slot, the KCCB
  write offset and the doorbell once per command. A batched post would tell
  the doorbell's cost apart from the KCCB command's. It is a follow-up only
  if the held arm below shows a per-command cost. It still makes the
  firmware handle N commands, so it is not the product shape.
- **The re-post** (`pvr_render_wait_repost`) re-sends a KICK the firmware
  already has. It is the experiment's safety net (below), not a form of
  submit.

## The experiment

### Why case 219 as landed does not decide it

Case 219 (`driver/tools/pvrlab/gl_test.c:20555-20717`) times arrangement
"two" (render X, render Y, glFinish) against "one" (X, glFinish), 20 reps
each (`:20576`, `:20700-20704`). In both arrangements each render is one
KCCB command, so `two - one` is the price of one more render *and* one more
command. `terrain-board-readout.md:156` reads "the second render posts its
geometry fence much sooner" as per-command evidence. That does not follow:
both hypotheses predict the same `two` arm. What separates them is an arm
in which the second render has **no command of its own**.

### The diff sketch: one experiment commit, never merged

All of these paths belong to running plans (#325: `driver/src/pvr_render.*`,
`driver/rust/src/gpu/pvr/**`; `gl_test.c` is shared). The commit goes on an
experiment branch from an origin/main commit and is built with
`--allow-local`, as `pvr-rering-knob.md` ("How to set it, per image", A) and
`pvr-mip-tail-e1.md` ("(3) The board procedure") do.

**1. `driver/src/pvr_render.c` and `pvr_render.h`: a hold counter, no layout
change.** It is applied after `rr_sync_cfg()` in `pvr_render_submit`
(`pvr_render.c:802-816`; `rr_sync_cfg` `:653-672` copies
`pvr_render_combined_kick` into `RR.cfg.combined_kick` on every submit and
wait, so the bit cannot outlive the submit):

```c
/* EXPERIMENT (never merge): hold the KCCB kick of the next N submits. Their
 * client-CCB commands are written; the next submit not held posts one kick
 * whose write offsets cover them (docs/notes/pvr-chain-one-submit.md). */
uint32_t pvr_render_hold_next;
uint32_t pvr_render_holds;
...
	rr_sync_cfg();
	if (pvr_render_hold_next) {
		pvr_render_hold_next--;
		RR.cfg.combined_kick |= 2u; /* bit 1: write the commands, post nothing */
		pvr_render_holds++;
	}
	rc = yd_pvr_render_submit(pvr_km(), &RR, k, fence_out);
```

Add the two `extern` lines beside `pvr_render_combined_kick`
(`pvr_render.h:255`). `YdPvrRenderCfg.combined_kick` stays a `u32`
(`render.rs:278`; C mirror `driver/rust/include/yakogl_drivers.h:1346`,
size assert `:2244`). No offset moves.

**2. `driver/rust/src/gpu/pvr/render.rs` `submit`, `:1629-1644`.** Test bit
0 for "combined" and bit 1 for "hold". Keep `last_kick` (and so the 20 ms
re-post) as it is:

```rust
let hold = r.cfg.combined_kick & 2 != 0;
r.last_kick_n = 0;
if r.cfg.combined_kick & 1 != 0 {
    /* ck as today */
    r.last_kick[0] = ck;
    r.last_kick_n = 1;
    if !hold { kccb_post(k, &ck); }
} else {
    /* gkick, fkick as today */
    if !hold { kccb_post(k, &gkick); kccb_post(k, &fkick); }
}
```

Nothing else changes. The next submit that is not held builds its kick from
`r.gwo` and `r.fwo`, which already include the held render's bytes
(`:1468`, `:1594`, `:1620-1621`). If a wait comes first, while a hold is
still pending, `wait_for_at`'s 20 ms re-post (`:2382-2387`) posts the held
kick, which covers everything written up to it. Such a stray wait therefore
costs 20 ms and one count in `pvr_render_reposts`; it does not hang. The
arm then reads void (below). `kick_times::posted` (`:1665`) stamps a held
render at its write, not at its post. Only the last render of an arm is
read (`pvr_render_last_kick_us`, `pvr_render.c:823-826`), and it is always
posted.

**Host test (in `submit_tests`, `render.rs:3734`).** The rig already has
the KCCB control and 8 slots (`KCTL`, `KCMD`, `:3753-3754`), and
`a_size_with_every_member_in_flight_takes_a_second_pair` (`:4011-4038`)
shows four kicks of one size in flight with no wait.

```rust
/// A held kick posts nothing; the next kick's one KCCB command carries
/// write offsets past both renders; the CCB bytes are today's.
#[test]
fn a_held_kick_goes_out_under_the_next_kicks_offsets() {
    let mut t = rig(0);
    let (mut f1, mut f2) = (0u32, 0u32);
    t.r.cfg.combined_kick = 1 | 2;
    assert_eq!(submit(&mut t.k, &mut t.r, &kick(1), &mut f1), 0);
    assert_eq!(t.word((KCTL + CCB_CTL_WRITE_OFFSET as u32) as usize), 0); // no post
    t.r.cfg.combined_kick = 1;
    assert_eq!(submit(&mut t.k, &mut t.r, &kick(2), &mut f2), 0);
    assert_eq!(t.word((KCTL + CCB_CTL_WRITE_OFFSET as u32) as usize), 1); // one command
    let s = KCMD as usize; // slot 0
    assert_eq!(t.word(s + KCCB_CMD_TYPE), KCCB_CMD_COMBINED_TA_3D_KICK);
    assert_eq!(t.word(s + KCCB_COMBINED_TA + KCCB_KICK_CWOFF - KCCB_KICK_CONTEXT), 2 * GEOM_BYTES[0]);
    assert_eq!(t.word(s + KCCB_COMBINED_3D + KCCB_KICK_CWOFF - KCCB_KICK_CONTEXT), 2 * FRAG_BYTES[0]);
    // and the two renders' commands are the ones without_the_knob_the_commands_are_todays pins
}
```

Run it with `cargo test -p yakogl-drivers --features pvr --lib submit_tests`
(the crate's `pvr` feature, as `tools/check.sh:2713` builds it). Run it
again with `--features pvr,pvr-kick-times`, the board image's set with kick
times on. The field offsets are in `driver/rust/src/gpu/pvr/fwif.rs:357-365`.

**3. `driver/tools/pvrlab/gl_test.c` case 219.** Same case number, new arms:

- `KICKPAIR_REPS` 20 -> 100 (`:20576`). `now_ms` resolution over a run is
  then 0.01 ms a rep.
- Four targets instead of two (`:20657-20693`), at a size held in two
  statics `kp_w, kp_h` that `kickpair_render` uses for its viewport (now
  64x64, `:20588`).
- `kickpair_arrangement(name, renders, held, ...)`, where `held` sets
  `pvr_render_hold_next = renders - 1` before each rep's renders (inside the
  loop at `:20605-20609`, after the previous rep's `glFinish`). It reads
  `pvr_render_holds` before and after the run.
- The arms, once at 64x64 and once at 400x300 (the size every 800x600
  chain's level kick uses: `record_mip_level` takes level 1's size as the
  frame, `src/pvr/backend.rs:7179`, called from `mip_chain_gpu` at `:7310`):
  `one` (1 render), `two`, `two-held`, `four`, `four-held`. Each arm keeps
  the existing two lines (`:20630-20632`, `:20640-20651`), with the size
  after the name and two fields added:

  ```text
  gl_test: kickpair: four-held 400x300: 100 reps, X ms a rep, 4.00 kicks a rep, reposts 0, holds 300 (want 300)
  gl_test: kickpair: four-held 400x300: last kick, mean us over 100 reps: submit ..; post->geom ..; post->frag ..; prev frag->frag ..; wait ..;
  ```
- An untimed pass of four renders before each size's arms, as today
  (`:20700-20702`).
- Every render samples the 64x64 `src` and none samples another arm
  target, so no GL-core settle wait comes between renders. Four renders of
  one size take the size's four HWRT members with no wait (`HWRT_MEMBERS`,
  `render.rs:95`; `hwrt_get` `:713-781`). The backend's frame-set wait
  (`src/pvr/backend.rs:6115`, kick N waits for N - 4) meets only kicks of
  the previous rep, which `glFinish` completed. The geometry UFO ring
  throttle (16) is not reached either.
- Checks: the existing 1-3, plus 4, `holds == want` in each held arm.

**4. `driver/samples/zephyr/gl_test/prj.conf`:** add
`CONFIG_YAKOGL_DRV_GPU_PVR_KICK_TIMES=y` (`driver/zephyr/Kconfig:198`,
feature `pvr-kick-times`, `driver/zephyr/CMakeLists.txt:194-195`). No
`tools/cts` script sets it. Without it the `last kick` lines say "not
built" and the ms lines alone still decide. The 20 us poll is already the
AM62P default (`driver/zephyr/Kconfig:183-185`, `render.rs:2034-2037`).

**If the gl_test part is ever landed** without the driver hold (the held
arms then print "holds 0 (want N)" and are void), it should go to case 219
as above. A separate case would take the next free number: 248 at
`975a6792` and at origin/main `2be14a44`. Cases 244-247 are taken, and no
visible branch adds a number above 247.

### What each hypothesis predicts

Definitions, from one session's 400x300 lines (the 64x64 lines give the same
quantities at a size with almost no tiles):

- `c = (four - one) / 3`: the marginal cost of a render plus its command,
  queued.
- `s = (four - four-held) / 3`: what one KCCB command costs beyond its
  render.
- `c2 = two - one` and `s2 = two - two-held`: the same at a queue depth of
  2.

Reference figures: a 64x64 kick posted to an idle GPU reaches its geometry
fence in 120-140 us and its fragment fence in 220-230 us
(`pvr-fence-wait.md:175-178`). A queued kick in terrain or case 216 costs
about 1.1 ms of frame with the GPU busy about 0.19 ms of it
(`am62p-slow-terrain.md:201-208`). The held arms start the GPU later, by the
CPU's recording time of the renders before the last. That biases
`four-held` upward by at most 3 x the board's per-render GL time (about 0.05
to 0.15 ms; the `submit` field and `one - post->frag` bound it).

| hypothesis | `four - one` (400x300) | `four - four-held` | `s` | the last kick of `four-held` |
|---|---|---|---|---|
| **H-cmd**: the ~0.9 ms is per KCCB command | about 3 x 1.1 = 3.3 ms | about 2.7 ms | about 0.9 ms | `prev frag->frag` near the render's own GPU time (under 0.3 ms), against about 1 ms in `four` |
| **H-render**: it is per render, whatever carries it | about 3.3 ms | about 0 (within the bias) | under 0.15 ms | `prev frag->frag` about the same as in `four` |
| **H-none**: a 4-deep back-to-back queue does not show it | under 1 ms | small | small | `post->geom` near the idle 120-140 us plus the geometry ahead of it |

### The decision (record values)

Void first: any arm with `reposts` other than 0, a held arm with `holds`
not equal to `want`, `kicks a rep` not equal to the arm's renders, a GL
error (check 3), or a GPU reset in the log. A void run is rerun. If it
repeats, the hold meets a wait the arms were built to avoid, and that is a
finding for this note.

Then, on the 400x300 block:

1. **Per command (build candidate 4):** `s >= 0.5 ms` and `s >= 0.6 c`.
   Predicted terrain gain (each of 43 level kicks a frame costs about 1.1
   ms, `am62p-slow-terrain.md:288-302`): **-30 x s** with at most 4 renders
   a command, 13 commands a frame instead of 43 (product step 1 below), and
   **-38 x s** with one command a chain, 5 a frame (step 2). At `s` = 0.9
   that is -27 to -34 ms of 83.
2. **Per render (drop candidate 4):** `c >= 0.5 ms` and `s <= 0.15 ms`. No
   submit change gains anything. The lever is fewer renders: candidate 2,
   three levels a render (`pvr-mip-three-levels.md`), at `c` a render
   removed.
3. **Neither:** `c < 0.3 ms`. Back-to-back renders of one size do not show
   the ~1.1 ms, so terrain's per-kick price comes from something the arms
   leave out: a level sampling the previous level, the frame-set waits, or
   the once-a-frame 20 ms fence (candidate 1). Candidate 4 gains at most
   38 x `c`. Drop it and read `pvr-waited-kick.md`'s and
   `pvr-frameset-wait.md`'s counters instead.
4. **Between** (`0.15 < s < 0.5`): part of the cost is per command. Predict
   -30 x `s` to -38 x `s` and weigh that against the work in steps 1-2.

The 64x64 block says whether the outcome depends on tile count. Comparing
`s2` with `s` says whether depth 2 already shows it, which would also settle
case 219's own reading.

## The board procedure

Leases: `target:am62p` with `target:am62p-console` (shared with #313 and
#325). One image, one commit, all arms in one run. The comparison is inside
the run, so no "before" image is needed.

```text
git fetch origin
git checkout -b exp-chain-one-submit origin/main        # note the sha: <main>
# apply the sketch: pvr_render.c, pvr_render.h, render.rs (+ the host test),
#   gl_test.c case 219, driver/samples/zephyr/gl_test/prj.conf
cargo test -p yakogl-drivers --features pvr --lib submit_tests
cargo test -p yakogl-drivers --features pvr,pvr-kick-times --lib submit_tests
git commit -am 'EXPERIMENT: hold a render'\''s KCCB kick; case 219 held arms (never merge)'
tools/cts/pr-session.sh chain1-a --knob off --cases '219' --allow-local --dry-run
tools/cts/pr-session.sh chain1-a --knob off --cases '219' --allow-local
tools/cts/pr-session.sh chain1-b --knob off --cases '219' --allow-local   # a second run, for the spread
```

- `--knob off` is main's partial-render setting
  (`tools/cts/pr-session.sh:16`, `:105`). `--cases` sets
  `CONFIG_GL_TEST_CASES` (`:113`). `--allow-local` is needed because the
  commit is not on origin/main (`:85-91`). The records go to
  `tools/cts/log/<date>-pr-chain1-a/` and `-chain1-b/`.
- Read from the console log: the ten `gl_test: kickpair: <arm> <size>: ...
  ms a rep ...` lines and the ten `last kick` lines of each run, plus
  `gl_test: case 219` PASS (checks 1-4).
- Record: a section added to this note giving `<main>`, the experiment sha,
  the two records paths, the ten ms values a run, `c`, `s`, `c2` and `s2`
  at both sizes, and the outcome from "The decision". Put the same line in
  the #1199 comment. The glmark2 rows of `RESULTS.md`, `STATUS.md` and
  `README.md` are not touched (#317's table).

## If it is per command: the product shape (not built here)

- **Step 1, up to 4 renders a command, no new UFO.** `record_mip_level`
  marks every level kick of a chain but the last as held. A flag in
  `struct pvr_render_kick` or the OSAL submit carries the mark to `submit`
  (`src/pvr/backend.rs:7179`, `:7310`; `driver/src/pvr_render.h`). Then
  `wait_for_at` (`render.rs:2344`) must, on entry, post a pending held kick
  whose fence it is asked for. The frame-set wait for kick N - 4
  (`backend.rs:6115`) and the HWRT member wait (`render.rs:759-781`)
  otherwise stall 20 ms on a held kick. A chain of 9 becomes 3 commands
  (4 + 4 + 1) and a chain of 8 becomes 2, so terrain posts 13 commands for
  its 43 levels.
- **Step 2, a whole chain a command.** The fifth render of a size reuses a
  member whose last render is in the same, unposted command, so the host
  cannot wait for it. Give each HWRT member a UFO word written by an UPDATE
  after every 3D that uses the member (value = seq). Give the GEOM of the
  member's next render a FENCE on that word equal to the member's
  `last_seq`. The word moves only with that member's renders, in order, so
  the equality test (`render.rs:546-555`) holds exactly when the previous
  user's fragment phase is done. `hwrt_get` then hands the member out with
  no host wait. The backend's frame sets must not wait on a held kick
  either: either more sets for chain levels, or a chain recorded into one
  set (`pvr-frameset-wait.md`: the depth of 4 is a choice).
- Neither step changes a command the GPU runs, only when the firmware is
  told about it, so no ES 2.0 answer or glmark2 picture changes
  (`tests/es2_report_guard.rs` is unaffected). Both touch
  `driver/rust/src/gpu/pvr/render.rs`, `driver/src/pvr_render.*` and
  `src/pvr/backend.rs`, held by #325 and #301. They go after those plans
  release the files, small, rebased on the then origin/main.
