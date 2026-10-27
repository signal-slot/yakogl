<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP: how 33 glmark2 scenes and the cluster frame rate reach the host

Design only (2026-10-17). It settles the design half of items 3 and 4 of
`docs/notes/stage2-bench-gap.md` ("Work that closes stage 2"); no code
changed. The implementation is the held item `imx8mp-gm-record`, told what
to build in the hand-over note of `d9e098cf`.

## The problem

The Verdin iMX8M Plus on the Mallow carrier has no console (the debug header
X11 is unassembled, `driver/zephyr/lib/board/imx8mp/gc_panel_console.h`). A
run leaves one record, four 32-bit words, in the SNVS LPGPR0..3
(`driver/rust/src/board/record.rs`, "Format"; `gc_result.h`); every store
replaces the last. A glmark2 run therefore reports its score, its scene
counts and its GL error counters (kind GLMARK2) and nothing per scene: no
fps line, no `glmark2-frame:` line, no `glmark2-picture:` line
(`driver/docs/glmark2.md`, "The picture value"). The cluster run reports
its PROBES verdict but not its frame rate: the `throughput:` line needs
FINGERPRINT, PERF page 0 and APP together (`record.rs`,
`Throughput::from_records`), and only the last of the four records survives.

What the host needs per scene is what `tools/cts/common-tally.py` reads:

- the result line `<info>: FPS: N FrameTime: X ms` (`GM_RESULT_RE`,
  `GM_FPS_RE`), or `<info>: Set up failed` / `<info>: Unsupported` for a
  scene that did not run: `glmark2_speed` takes the FPS integer from it and
  derives ms/frame as 1000 / FPS;
- the frame line `glmark2-frame: <info>: N frames; ...` (`GM_FRAME_RE`):
  `glmark2_scenes` uses it to bound a scene, `glmark2_speed` reads its
  optional `gl` and `end waited` parts (`None` when absent, `-` in the
  table);
- the picture line (`GM_PICTURE_RE`, `PICTURE_VALUE_RE`): size, five probes
  as RRGGBBAA, crc16, crc16q, mean R G B, black. `judge_picture` compares
  the probes' and the mean's R, G, B within the table's tolerance (8), the
  size, and black; crc16 is named when it differs, never judged; alpha is
  never read.

`<info>` is the scene's info string (`[terrain] <default>`,
`[pulsar] light=false:quads=5:texture=false`: the scene name in brackets and
its options in name order).

## What a run can carry, measured

- **Payload of one record:** w1..w3 = 96 bits (w0 is magic, kind, tag).
- **One scene, as the console carries it:** 5 probes x 3 channels x 8 bits
  = 120, mean 24, crc16 16, crc16q 16, black 1, size 23, fps 16, frames 16
  = 232 bits: three records, not one. With the probes at RGB565 (the form
  kind PROBES already uses, `record.rs rgb565`): 80 + 24 + 16 + 16 + 1 +
  23 + 16 + 16 = 192 bits, two records exactly.
- **A run's cost.** Nothing in the tree measures a boot alone; two
  documented times bound it. A short gl_test image is back with its record
  "about 2 to 2.5 minutes" after the start (`tools/cts/README.md`,
  i.MX8MP); a full glmark2 run is "one boot of about 7 minutes"
  (the hand-over of the time, the `gle0` runs) for 33 scenes of 10 s plus set-up
  (`driver/docs/glmark2.md`, "The scenes", and `RUN_BOUND=960`: "about 6-7
  minutes" for the benchmark itself). Both fit a boot-and-return overhead
  of 60 to 90 s around the image's own run; the older crash route measured
  "linux back after" 92-116 s with a short run included
  (`docs/notes/imx8mp-netboot.md`, section 8). This note takes **B = 90 s**
  (the conservative end) and **S = 11 s a scene** (10 s of frames, about
  1 s of set-up: shader compiles take about 0.2 s each). Check: a full run
  is 90 + 33 x 11 = 453 s, 7.5 min, which agrees with the documented 7.
  No board was run for this note.
- **Where bytes can live:** the LPGPR only. `gc_result.h` and
  `gc_panel_console.h` state that the SoC reset which brings Linux back
  clears DDR and OCRAM, that SRC_GPR8/9 belong to TF-A, and that the four
  LPGPR are the one place that keeps a value (checked 2026-09-23). The
  Zephyr image has no USDHC, Ethernet or USB driver (nothing under
  `driver/zephyr`, `driver/samples` or `driver/rust` names one), so no other
  storage is reachable from the image.

## The four ways

| | runs for 33 scenes | wall time | bytes a scene | gives the tally its lines |
|---|---|---|---|---|
| (a) one scene per run, selector = scene | fps only: 33; fps and picture, lossless: 99 (3 records a scene) | 33 x 101 s = 56 min; 99 x 101 s = 2.8 h | 12 (4 used); 36 | fps yes; the 232-bit picture never fits one record |
| (b) a log in DRAM, read back after the reset | 1 | 7.5 min | about 300 (the three text lines) | it would, but the log is gone |
| (c) packed pages, one page a run | fps: 11 (3 scenes a page); fps and picture: 66 (2 pages a scene) | 11 x 123 s = 23 min; 66 x 101 s = 1.9 h | 4; 24 | yes, probes at RGB565 |
| (c') as (c), but the full list runs every time | 11; 66 | 11 x 453 s = 83 min; 66 x 453 s = 8.3 h | 4; 24 | yes |
| (d) a console path | 1 | 7.5 min | about 300 | no: a photograph |

Workings: a run costs B + S x scenes run. (a) 90 + 11 = 101 s. (c) an F
page runs three scenes, 90 + 33 = 123 s; a P page runs one, 101 s. (c'),
(b) and (d) are today's full run, 453 s.

**(a) One scene per run** is the cheap path for fps alone and cannot carry
a picture in one record; three records a scene (lossless) is 99 boots,
2.8 h of board time for one table. The frames and fps of three scenes fit
one record, so (a) is (c)'s F page with two of its three slots empty.

**(b) A DRAM log** is what the dEQP sample's files look like but not what
they do. `driver/samples/zephyr/deqp/src/deqp_log.c` turns the QPA stream
into console lines and calls `deqp_snvs_progress`; `deqp_snvs.c` keeps a
running tally in the SNVS (kind DEQP, or DEQP_CASES with two bits a case up
to 32 cases) and names failing cases six at a time in pages (kind
DEQP_FAILS, selector `0x8000 | page << 8 | index`, one boot a page,
`tools/board.sh name_pages`). It never stores a log in memory: there is
nowhere for it. DDR does not survive the reset (the fact above). Rejected.

**(c) Pages** is the form DEQP_FAILS and PERF (`0x1000 | page`) already
have: one boot, one page, chosen by the runner's selector. Packing makes a
scene's 192 bits two pages and three scenes' fps one page. The benchmark
list is cut to the page's scenes, so a boot costs the scenes it reports and
not the other 30 (c'): the same records in 23 min instead of 83, and 1.9 h
instead of 8.3.

**(d) A console.** The panel console (`gc_panel_console.c`) renders printk
into the LCDIF's framebuffer: "a photograph of the panel is then the console
log". It is compile-tested only, nothing in the tree reads text out of a
photograph, and 33 scenes' three lines are about 10 KB, many screens. The
UART header is unassembled. Rejected.

## The choice: (c), packed pages behind one selector family

The run count has a floor: information / 96 bits. A scene's picture and fps
at RGB565 probes are 192 bits, two records; 33 scenes are 66 records. A
bit-stream across scenes would carry the pictures alone in 55 records (160
bits a scene) at the price of a page that means nothing by itself; 11 boots
(19 min) are not worth it. Two sessions, one kind:

- **The fps session (F pages):** 11 boots, 23 min. Three scenes a page: the
  FPS integer and the frame count.
- **The picture session (P pages):** 66 boots, 1.9 h. Two pages a scene; P1
  carries the FPS integer and the frame count too, so a picture session
  also gives the fps.

The score, the scene counts and the GL error counters stay with the normal
full run (kind GLMARK2, with GL_ERROR pages for #814). The pages add the
per-scene values beside it; a complete GM row is the full run plus one of
the sessions. The cluster's rate is one more boot of the cluster image with
its own selector (kind RATE).

### The selector

`SELECTOR_MAGIC | index`, as today (`record.rs`; `tools/board.sh snvs_clear`
writes it when `SEL` is set; `gc_result.c result_map` reads it before the
first record). Bit 0x400 is free: the flags in use are 0x8000 DEQP_SEL_FAILS,
0x4000 DEQP_SEL_PROBE, 0x2000 DEQP_SEL_STATUS, 0x1000 PERF_SEL (with the
KICK_DIFF and MSAA tags of `docs/notes/gc-kick-diff.md` and
`gc-msaa-record.md` in its low 12 bits), 0x800 GL_ERROR_SEL, 0x200 and
0x100 SWEEP_SEL_LIST / _LEAKS. The family is

```text
GM_SEL (0x400) | t << 6 | n            t = (sel >> 6) & 3, n = sel & 0x3f
  t = 0   F page n, n = 0..10          scenes 3n, 3n+1, 3n+2 (page 10 = scenes 30..32)
  t = 1   P0 of scene n, n = 0..32     the probes and crc16
  t = 2   P1 of scene n, n = 0..32     crc16q, fps, frames, mean, black, size
  t = 3   RATE (n = 0)                 the cluster sample's frame rate
```

A sample recognises the family by `sel & ~0xff == GM_SEL`, as glmark2
recognises `GL_ERROR_SEL` (`store_gl_error_record`); the record's tag is
`sel & 0xff`. No other image reads a GM selector: the dEQP image tests its
own bits (0x8000, 0x4000, 0x2000) and sees none set.

### The record kinds

Two rows in the `kinds!` table of `record.rs`, 24 and 25 (16, 17 and 19
stay reserved, `RESERVED_KINDS`). Both are `Tag`: the sequence number is
untouched. `classify` already makes any unknown-to-it result kind a
`Class::Verdict` (its default arm), so `tools/board.sh` ends the row `done`
with `result=record:GM_PAGE` or `record:RATE`. In `recordcmd.rs status()`,
GM_PAGE is `n/a` (a verdict of completion alone, as PICTURE is) and RATE
carries `STATUS_PASS` / `STATUS_FAIL` in its tag, as PROBES does.

**GM_PAGE = 24, tag = `sel & 0xff`** (t = tag >> 6, n = tag & 0x3f):

```text
t = 0, F:   w1 = scene 3n:    fps | frames << 16
            w2 = scene 3n+1:  the same
            w3 = scene 3n+2:  the same
t = 1, P0:  w1 = c  | tl << 16          each probe RGB565: R 5 bits at 11, G 6 at 5, B 5 at 0
            w2 = tr | bl << 16
            w3 = br | crc16 << 16       crc16 = the sample's crc16 of the 40x30 grid, exact
t = 2, P1:  w1 = crc16q | fps << 16
            w2 = frames | mean_r << 16 | mean_g << 24
            w3 = mean_b | black << 8 | W << 9 (11 bits) | H << 20 (12 bits)
```

- `fps` is the FPS integer upstream prints on the scene's result line:
  `ceil(1 / stats.average_frame_time)` of `Scene::stats()`
  (`main-loop.cpp:178` at the pinned 22c527cb), computed the same way from
  the same value, so the decoded line equals a console's; saturating at
  `GM_FPS_MAX` 0xfffd. Two codes say the scene did not run:
  `GM_FPS_FAILED` 0xfffe (set-up failed) and `GM_FPS_UNSUPPORTED` 0xffff,
  both with `frames` 0, so a page run keeps the per-scene verdict its
  GLMARK2 record (which the page replaces) would have summed.
- `frames` is the scene's frame count (`s_cost.frames`), saturating at
  0xffff. It gives the `N frames;` of the frame line; the `gl`, `swap` and
  `show` means are not carried (below, "What it does not do").
- Probes are the sample's `Picture::probe[0..4]` (c, tl, tr, bl, br) at
  RGB565 by `rgb565` of `record.rs`. The decoder widens by shift alone
  (`r5 << 3`, `g6 << 2`, `b5 << 3`, low bits 0), so a probe's printed value
  is the floor of its 565 step: a real change of one step moves the
  printed R or B by exactly 8 and G by 4, within the table's tolerance 8;
  two steps (16) is a mismatch. Alpha is not carried and is printed `00`.
- `crc16`, `crc16q`, `mean` and `black` are the sample's `Picture` fields as
  they are (`capture_picture`). `black` stays a flag; the grid is not
  carried.
- `W = H = 0` in P1 is the `absent (the readback failed)` picture; P0's
  words are 0 then. A window that does not fit 11 and 12 bits saturates
  (this image's window is 800x600, `driver/docs/glmark2.md`, "Why 800x600").
- A scene's picture and fps are P0 + P1: 96 + 96 bits, two boots.

**RATE = 25, tag = status (1 pass, 2 fail):** the cluster sample's frames
and time. `w1` = frames presented, `w2` = the milliseconds they were
presented in (APP's w2 and w3 as `cluster_board_finish` fills them,
`rec.app[1]` and `rec.app[2]`), `w3` = `W << 16 | H` (`size_word`).
`Throughput::from_records` takes a RATE record as an alternative to the
FINGERPRINT + PERF + APP trio (`draw_tenths` None: the draw mean is PERF's
and is not carried), so `yakogl-tools record` prints the same
`throughput: WxH: F frames in M ms, X.X fps, Y.Y Mpx/s` line for it
(`write_throughput`, without the draw part).

### The writers

- **glmark2** (`driver/samples/zephyr/glmark2/src/main.cpp`). In `run()`,
  after `collection.populate_from_options()`: `board_selector()` (already
  read by `store_gl_error_record`); for the GM family the benchmark list
  given to `RecordingLoop` is cut to the page's entries (F: list entries
  3n..3n+2; P: entry n). `RecordingLoop` already has each scene's frame
  count (`s_cost.frames`, `print_cost`), its set-up status
  (`scene_setup_status_`) and its picture (`picture_`, `capture_picture`);
  `log_scene_result` also keeps the scene's FPS integer and these fields in
  a small per-run array (up to three entries). After the loop and after the
  GLMARK2 record and `store_gl_error_record`, a `store_gm_page()` beside
  `store_gl_error_record` stores the page with
  `board_result_tag(YK_RECORD_KIND_GM_PAGE, sel & 0xff, w1, w2, w3)` from
  the encoders of `record.rs` (`gm_page_f_words`, `gm_page_p0_words`,
  `gm_page_p1_words`, exported through `yakogl_drivers.h` as
  `yd_record_glmark2_words` is). The page is the run's last record, after
  GLMARK2, so the i.MX8MP keeps it. The cut list makes that run's GLMARK2
  record mean "ran these scenes"; nothing reads it. Entry numbers in the
  `glmark2-mem:` lines and the GL error tags are the cut list's; they are
  console-only and the i.MX8MP has none.
- **cluster** (`driver/samples/zephyr/cluster/src/main.c`, `main`, after the
  PROBES and CHECK records): if `board_selector()` is `GM_SEL | 3 << 6`,
  store RATE last with `rec.app[1]`, `rec.app[2]` and
  `size_word(WIN_W, WIN_H)`, tag = the run's `status`. On a failed run the
  RATE then stands where CHECK would have; the normal CL run (no selector)
  keeps CHECK as today.
- Console boards are untouched: `board_selector()` is -1 without a runner's
  selector, the lists run whole, and the `glmark2-frame:` /
  `glmark2-picture:` lines stay as they are.

### The readers

- **`driver/rust/src/board/record.rs`:** the two `kinds!` rows; `GM_SEL`,
  `GM_SEL_T_SHIFT` 6, `GM_SEL_F` 0 / `GM_SEL_P0` 1 / `GM_SEL_P1` 2 /
  `GM_SEL_RATE` 3, `GM_PAGE_SCENES` 3, `GM_FPS_MAX`, `GM_FPS_FAILED`,
  `GM_FPS_UNSUPPORTED` in `values!` (so the C header has them); the three
  `gm_page_*_words` encoders, `rate_words`; a `GmPage` decoder
  (`from_words`: F with its three `(fps, frames)`, P0, P1); `describe` text
  ("GM_PAGE (24) F page 2: scene 6 [shading] shading=blinn-phong-inf fps 61,
  613 frames; scene 7 ...; scene 8 unsupported"); the RATE arm of
  `Throughput::from_records`; and `GLMARK2_DEFAULT_INFO`, the 33 info
  strings of `GLMARK2_DEFAULT_LIST` in list order (the scene name in
  brackets, its options sorted by name), so one table names a scene for
  every reader. The header `driver/zephyr/lib/board/include/yakogl_record.h`
  and `driver/rust/include/yakogl_drivers.h` are regenerated in the same
  commit (the gate compares the bytes; `YK_RECORD_BLESS=1 cargo test -p
  yakogl-drivers board`).
- **`tools/yakogl-tools/src/recordcmd.rs`:** `record --gm-page w0 w1 w2 w3`
  prints one `key=value` line per scene of the page, so Python never repeats
  the bit layout: F `scene=6 info=[shading] shading=blinn-phong-inf fps=61
  frames=613` (or `fps=failed` / `fps=unsupported` with `frames=0`); P0
  `scene=4 info=... c=ff8000 tl=... tr=... bl=... br=... crc16=3a1c`
  (RGB as widened above); P1 `scene=4 info=... crc16q=... fps=... frames=...
  mean=12,40,200 black=no size=800x600` (or `size=absent`). `status()` gets
  the RATE arm; `record w0..w3` on a RATE record prints its `throughput:`
  line. `--line` therefore reads `status=n/a` for GM_PAGE and `pass` /
  `fail` for RATE, which is what `board.sh` turns into `done`.
- **`tools/board.sh`:** no change to `run`: `SEL=$((0x400 | t << 6 | n))
  tools/board.sh imx8mp run IMG OUT NAME` already writes the selector into
  the cleared record and keeps `snvs.txt`; the verdict line reads
  `verdict=done result=record:GM_PAGE`. One new verb, `board.sh imx8mp gm
  IMG OUT fps|pictures|rate`, loops the selectors with `tree_run_one` (its
  retry rule; resumable: `finished_run` skips a run that has `INDEX.txt`
  for the same image): run names `gm-f00 .. gm-f10` (fps), `gm-p00a`,
  `gm-p00b` .. `gm-p32b` (pictures: a = P0, b = P1), `cl-rate` (rate, the
  cluster image). It ends by calling `common-tally.py --gm-pages OUT`.
  `tools/board_selftest.sh` runs the verb dry (`--dry-run`), as it runs
  every verb.
- **`tools/cts/common-tally.py`:** `gm_pages(outdir)` finds those run
  directories by their name (`RUN_RE`), reads each `snvs.txt` through
  `yakogl-tools record --gm-page`, and writes the lines below in list
  order to `<outdir>/glmark2-pages.txt`, which `--gm-pages <outdir>` also
  prints. That file is a console as far as the existing readers are
  concerned: `--gm-table`, `--gm-diff`, `--gm-pictures` and
  `--gm-picture-lines` take it as their `<console>` argument unchanged, and
  `tally imx8mp <outdir>` uses it as the GM row's console for
  `glmark2_scenes`, `glmark2_pictures` and `judge_pictures` when it exists
  beside the rows (so `board.sh imx8mp gm` is given the common set's
  outdir). The CL row's `throughput:` comes from the `cl-rate` run's record
  (`record_throughput`, beside `console_throughput`) instead of
  `throughput absent (no console records)`.

The lines, one set per scene whose page(s) exist, in list order:

```text
glmark2-pages: decoded from GM_PAGE records in <outdir>: F pages 0-10; P pages for scenes 0-32; missing: none
[terrain] <default>: FPS: 6 FrameTime: 166.667 ms
glmark2-frame: [terrain] <default>: 61 frames; us per frame: not carried (GM_PAGE)
glmark2-picture: [terrain] <default>: 800x600; probes c 10284000, tl ..., tr ..., bl ..., br ...; crc16 3a1c, crc16q 7f02; mean 12 40 200; black no
```

- The header line matches no tally regex (they key on `glmark2-frame:`,
  `glmark2-picture:`, `glmark2-mem:`, `glmark2: score` and `: FPS: `) and
  says where the lines came from; it names every page that is missing.
- `FrameTime` is `1000 / fps` with three decimals, derived: upstream prints
  the mean frame time itself (`1000 * stats.average_frame_time`), which the
  page does not carry. Nothing reads it: `glmark2_speed` takes the FPS
  integer and derives ms/frame as 1000 / FPS itself, as on a console board.
  The frame line's `gl` part is absent, so `gl ms` is `-`.
- A scene coded failed or unsupported gets `<info>: Set up failed` or
  `<info>: Unsupported` and no frame or picture line, as the board prints
  none for it (`print_cost` returns on 0 frames, `print_picture` on a
  set-up status other than success).
- A picture line needs both pages. With P1 but no P0 (or the reverse) the
  line is `glmark2-picture: <info>: absent (page P0 of scene 7 missing)`,
  which the tally judges `absent` (amber); a value is never invented.
- The i.MX8MP column of `tools/cts/glmark2-pictures.txt` is made with
  `common-tally.py --gm-picture-lines <outdir>/glmark2-pages.txt imx8mp`
  from a session whose panel was looked at, as the table's rule says, so the
  board is judged against its own 565 floors; a column copied from another
  board is not valid there either.

### The test that pins it

`driver/rust` has no `tests/` directory: its tests are the `mod tests` of
`driver/rust/src/board/record.rs` and `driver/rust/src/board/mod.rs`,
run by `cargo test -p yakogl-drivers` (the gate's drivers rows). New tests:

1. `record.rs`, `gm_page_words_and_their_decode`: golden words for an F
   page (three scenes: one normal, one `GM_FPS_FAILED`, one saturated at
   `GM_FPS_MAX` with 0xffff frames), a P0 page (known probes; the 565 round
   trip of 0, 8, 248 and 255 to 0, 8, 248 and 248), a P1 page (mean, black,
   800x600; `W = H = 0` as absent; a saturated size), and the `describe`
   text of each.
2. `record.rs`, `rate_words_give_the_throughput_line`: a RATE record and the
   FINGERPRINT + PERF + APP trio with the same frames, ms and size give the
   same `throughput:` line up to the draw part; a RATE with 0 ms gives none.
3. `record.rs`: the kinds test becomes `kinds_are_one_to_twenty_five...`
   (the reserved list unchanged); `GM_SEL` shares no bit with the other
   selector constants (the loop of `gl_error_words_and_their_decode`,
   extended); `classify` of a GM_PAGE and a RATE is `Class::Verdict`.
4. `record.rs`, `glmark2_default_info_is_the_default_list`: 33 entries;
   each info string's scene name and option set equal its
   `GLMARK2_DEFAULT_LIST` entry's.
5. `mod.rs`, `c_header_is_generated_from_the_table`: unchanged, it fails
   until `yakogl_record.h` is regenerated in the same commit.
6. `recordcmd.rs`: `--gm-page` output for the three page forms; `--line`
   reads `status=n/a` for GM_PAGE and `status=pass` / `fail` for RATE.
7. `tools/cts/cts-selftest.sh` and `tools/cts/testdata/gm-pages/`: a fixture
   outdir of run directories (`meta.txt` and `snvs.txt`, built as the
   i.MX8MP fixtures of `common-tally.py --self-test` are) with an F page,
   a P0 + P1 pair, a P1 whose P0 is missing, a failed-scene code and a
   `cl-rate` RATE; the expected `glmark2-pages.txt`; checks that every line
   parses by `GM_RESULT_RE`, `GM_FRAME_RE` and `GM_PICTURE_RE`, that
   `--gm-table` lists the scenes with their ms/frame, that the 33 info
   strings equal the `host` column's of `glmark2-pictures.txt` in order,
   that a column built from the fixture's lines judges them `match`, and
   that `tally imx8mp` on the fixture reads the GM row's scenes and the CL
   row's `throughput:` from the pages. `tools/board_selftest.sh`: the dry
   `gm` verb prints the 11, 66 or 1 selectors.

## What it costs and what it does not do

- A picture session is 1.9 h of board time, an fps session 23 min, the rate
  one boot. A row that needs only the fps (an A/B of the screen buffers,
  `imx8mp-floor-*`) uses F pages. The picture session is for the final
  board session (`imx8mp-final-board`) and after a change that moves
  pictures.
- The pages come from cut lists: a scene runs as the first of one to three
  in a fresh image, with cold caches and a cold GPU clock that the 33-scene
  run has warm by then. The pages are an A/B tool between page runs of the
  same kind, not proof that the full run's score is the mean of them; the
  score stays the full run's.
- Probes are 565 floors on this board. The tolerance 8 was set for GPUs
  that differ by a few levels; here one 565 step is 8 (R, B) or 4 (G), so
  a scene is judged on its own 565 grid. If an i.MX8MP scene reads
  `mismatch` with a probe off by exactly 16, it moved two steps on the
  board: look at the panel before changing the tolerance.
- A page run's GLMARK2 record is replaced by the page: the run's score and
  its GL error counters are not read. The per-scene did-not-run codes keep
  the scene verdicts; the counters are the full run's.
- The `gl`, `swap` and `show` means of the frame line are not carried.
  PERF page 0 of `docs/notes/imx8mp-frame-floor.md` ("The frame-breakdown
  record") is the place for one scene's breakdown and keeps its own
  selector (`PERF_SEL` 0x1000).
- The cluster's `throughput:` line from RATE has no `draw ... ms mean`
  part: that is PERF page 0's value, a second boot if it is ever wanted.
