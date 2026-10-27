<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# dEQP-GLES3 on the Verdin i.MX8M Plus (GC7000UL): board recipe

Plan #3, items imx8mp-gles3-tree and stage1-imx8mp. The i.MX8MP has never run dEQP-GLES3. This is the
recipe for the first run. **The gles3 stage 1 and the 133-row table image are built (host only,
`ab43a20c5:docs/board-sessions/imx8mp-deqp-stage1-gles3.md`); nothing has run on the board yet.** The commands are
the existing gles2 ones plus the gles3 switch. The expected lines have the shape of real gles2 lines from
the i.MX8MP gles2 runs, which went through the same `board.sh` code path. The board session
(item board-imx8mp) replaces them with what it sees.

## What is in the tree

`tools/cts/imx8mp-gles3-tree.txt` has 133 table entries. Together they cover the 42,810 cases of
`tools/cts/gles3-mustpass.txt`, and each case is matched exactly once. The longest entry is 1,868
characters (the limit is 2,035: `arg[2048]` in `case_argument()`, `driver/samples/zephyr/deqp/src/main.cpp`).
The limit on entries is 256, because the selector is masked to 8 bits. The file is generated:

```sh
python3 -I tools/cts/imx8mp-gles3-gen.py > tools/cts/imx8mp-gles3-tree.txt
python3 -I tools/cts/check-imx8mp-gles3-tree.py -q
```

```
mustpass 42810, entries 133 (max 256), longest entry 1868 chars (max 2035), largest entry 1000 cases, matched exactly once 42810
OK
```

Without `-q`, the checker prints one line per entry: `entry <i> <chars> chars <n> cases <first pattern>`.
Row `e<i>` of the board run is entry `<i>`.

**Sizing.** The budget is mustpass cases per entry. One boot runs one entry, and the WDOG1 window is 600 s
(`CONFIG_GC_BOARD_WDOG_SECONDS=600`, `driver/samples/zephyr/deqp/boards/verdin_imx8mp_mimx8ml8_a53.conf`).

| group | cases per entry |
|---|---|
| shaders | 1,000 |
| shaders.random | 600 |
| other groups | 800 |
| ubo | 600 |
| fragment_ops | 500 |
| fbo, fragment_out | 400 |
| transform_feedback | 300 |
| texture | 150 |
| texture.shadow | 80 |
| texture.format | 40 |
| texture.size | 30 |
| texture.filtering.3d | 20 |

Where the numbers come from:
- `tools/cts/RESULTS.md` §1r (AM62P) ran up to 2,000 shader cases per image inside 600 s.
- On the AM62P, the image holding texture.filtering .. texture.size outlived the window. `filtering.3d.*`
  took about 19-25 s a case and ran in images of 22 cases. At 20 per entry that is about 500 s, before
  any slowdown on the GC7000UL.
- The GC7000UL's own GLES2 table ran up to 1,900 shader cases per entry (`shaders.loops.*`), but its texture groups
  needed 36-192 cases per entry (`tools/cts/imx8mp-full-tree-29460f96.results.tsv`).

These are estimates, not measurements. If an entry ends at the watchdog, split it: lower that group's
budget in `imx8mp-gles3-gen.py`, regenerate the table and run the checker again. You can tell a watchdog
end because the record's `begun` is less than the entry's mustpass count and `back_after` is near 600 s
plus the boot. Groups likely to need it: `texture.filtering.3d.*`, `texture.size.*`,
`fbo.blit.conversion.*` and `fragment_ops.blend.*`. The 2048x2048 cases can end a run with
GL_OUT_OF_MEMORY in the 64 MiB carveout (see the board conf).

`lifetime.*` (56 cases) is entry 22 on its own. On the AM62P its init dies, because transform feedback is
missing (RESULTS.md §1r), so isolating it means a crash there costs no other cases. The patterns are
dEQP filters over the whole dEQP tree, so a row can also begin a few cases that are not in the mustpass.

## Environment

The same as for gles2 (`tools/cts/imx8mp-env.sh`): `ZEPHYR_WS`, `ZEPHYR_VENV`,
`ZEPHYR_SDK_INSTALL_DIR` and `IMX8MP_OUT`. You also need `DEQP_STAGE1`. Give gles3 a directory of its own,
because the gles2 directory holds the gles2 libraries and stage 2 links what it finds there. On the board
side, set `BOARD_HOST=<user@host>` from the environment (never write it into a file) and
`BOARD_BOOT=uboot-mmc`.

## 1. Stage 1 (once per VK-GL-CTS revision, about 1.9 GB)

```sh
export ZEPHYR_WS=<west workspace> ZEPHYR_VENV=<workspace>/.venv \
       ZEPHYR_SDK_INSTALL_DIR=<zephyr-sdk-1.0.1> IMX8MP_OUT=<out>
tools/cts/imx8mp-deqp-stage1.sh gles3     # = DEQP_MODULE=gles3 DEQP_STAGE1=<out>/deqp-stage1-gles3
```

The `gles3` argument uses `$DEQP_STAGE1_GLES3` if set, else `$IMX8MP_OUT/deqp-stage1-gles3`. The script also
exports `ZEPHYR_SDK` (from `ZEPHYR_SDK_INSTALL_DIR`) and `-O2` flags, which `cts-build-zephyr.sh` needs.
Measured: 453 ninja steps, about 15 minutes on a 12-thread worker, 2.0 GB.

This exports `MODULE=gles3` to `tools/cts/cts-build-zephyr.sh` (VK-GL-CTS 93bca01, with the same patches as
gles2). The log goes to `$IMX8MP_OUT/deqp-stage1.log`, the same file as for gles2.

Expected: `rc=0`, and `<out>/deqp-stage1-gles3/build/modules/gles3/libdeqp-gles3-package.a` exists. Stage
2 refuses to run without it. With no `DEQP_MODULE` (or `DEQP_MODULE=gles2`) the script does what it always
did.

## 2. Build the table image (stage 2)

```sh
HEAD8=$(git rev-parse --short=8 HEAD)
tools/cts/imx8mp-deqp-build.sh --gles3 es3-$HEAD8 'dEQP-GLES3.info.*' --table tools/cts/imx8mp-gles3-tree.txt
# (--gles3 = DEQP_MODULE=gles3 and DEQP_STAGE1=$DEQP_STAGE1_GLES3, default $IMX8MP_OUT/deqp-stage1-gles3)
```

What this adds:
- `-DDEQP_MODULE=gles3 -DCONFIG_DEQP_REPORT_ES2=n -DCONFIG_YAKOGL_REPORT_ES2=n`. Both are needed. The
  sample's CMake refuses gles3 when `DEQP_REPORT_ES2` is on. `YAKOGL_REPORT_ES2` is `default y`
  (`zephyr/Kconfig`), so turning off only `DEQP_REPORT_ES2` (which drops its `select`) still leaves the
  library reporting OpenGL ES 2.0, and dEQP-GLES3 does not run on an ES 2.0 context.
- `CONFIG_DEQP_CASE_TABLE`: the 133 rows joined by `;`, written to `$IMX8MP_OUT/es3-$HEAD8.table.conf`.

Expected output:

```
imx8mp: 133 table rows -> <out>/es3-<hash>.table.conf
rc=0
check-deqp-images: the dEQP image recipes build what conformance needs
```

The last line comes from `check-deqp-images.py --config <.config> --gles3`. It checks three things: the
in-process compiler is present, there is no precompiled tier, and neither `DEQP_REPORT_ES2` nor
`YAKOGL_REPORT_ES2` is `y`. Kconfig writes a disabled bool as `# CONFIG_X is not set`, so there is no `=n`
line to grep for. A line `check-deqp-images: FAIL ... CONFIG_YAKOGL_REPORT_ES2=y` (or `CONFIG_DEQP_REPORT_ES2=y`)
followed by `imx8mp: warning: not a conformance image` means that switch did not take effect, and the image
would report ES 2.0.

The image is `$IMX8MP_OUT/es3-$HEAD8/zephyr/zephyr.bin`. Name it after the commit it is built from, because
results are reported against that hash.

**Footprint.** The gles2 table image was 221,282,304 bytes in the 256 MiB `a53_ddr_section`
(imx8mp-plan1.md). Measured for the gles3 table image: `zephyr.bin` 15,878,520 bytes (`es3-ba8b242a`, sha256 `8df88235…bc8bd2`), RAM about
221,976 KB of 256 MB (84.7 %), footprint checked by `board.sh` 227,311,616 bytes. Details:
`ab43a20c5:docs/board-sessions/imx8mp-deqp-stage1-gles3.md`.

## 3. Run the tree on the board

```sh
export BOARD_HOST=<user@host> BOARD_BOOT=uboot-mmc
TREE_IMAGE=$IMX8MP_OUT/es3-$HEAD8/zephyr/zephyr.bin \
  tools/board.sh imx8mp tree tools/cts/imx8mp-gles3-tree.txt records/imx8mp/es3-$HEAD8
# the same, with the table and the newest $IMX8MP_OUT/es3-*/zephyr/zephyr.bin filled in:
tools/cts/imx8mp-gles3-board.sh tree records/imx8mp/es3-$HEAD8
```

- **Preview:** `tools/board.sh --dry-run imx8mp tree ...` (with the same `TREE_IMAGE`; or
  `tools/cts/imx8mp-gles3-board.sh --dry-run tree <dir>`) prints the `DRY host$ ...` steps for every row and
  touches no board. Done for the built image (`es3-ba8b242a`, sha256 `8df88235…bc8bd2`): 133 rows
  `e00`..`e132`, selectors 0..132; see the session doc.
- **Subset:** `TREE_ROWS=0-9` or `TREE_ROWS=57,60-66` runs only those rows.
- **Resume:** a row whose run directory already has an `INDEX.txt` is skipped.

Row *i* is run `e<i>` (`e00` .. `e132`) with SNVS selector *i*. The image reads the selector from the
SNVS (`gc_result.h`), runs table entry *i*, and resets through WDOG1 when it is done or after 600 s.

- A refusal (rc 3) is retried 60 times, 30 s apart.
- A board that does not come back (rc 5) stops the chain.
- Budget: 133 boots of about 50-700 s each, so several hours.

### How records are read

The carrier has no UART. After every case the image rewrites its four-word SNVS DEQP record. After the
reset, `board.sh` reads the record back through Linux into `<run>/snvs.txt` and decodes it with
`yakogl-tools record --line`.

| word | contents |
|---|---|
| w0 | magic `594b` and the record kind in bits 8-15 (`08`, DEQP; the failure pages are `0a`) |
| w1 | `passed \| failed << 16` |
| w2 | `notsup \| other << 16` |
| w3 | `begun \| first-fail-CRC << 16` (`ffff` when nothing failed) |

A row's tally is `passed,failed,notsup,other,begun`. "No usable record" means the words were absent or
were not a DEQP record.

When a row has a failure, another status or a case without a result, `board.sh` boots it again as
`e<i>-p<page>` with selector `0x8000 | page << 8 | i`. Each of these boots reads six failing-case CRC-16s.
`NAMES=0` turns this off.

**GLES3 caveat (failing names).** `board.sh` maps each CRC to a name with `yakogl-tools record crc <c>`.
That command defaults to `tools/cts/gles2-mustpass.txt` and the prefix `dEQP-GLES2.`, so for a GLES3 tree
the names in `results.tsv` come out as `crc:<hex>`. The counts are not affected. To resolve a CRC by hand:

```sh
DEQP_CASELIST=tools/cts/gles3-mustpass.txt cargo run -q -p yakogl-tools -- record crc <hex> dEQP-GLES3.
```

Then keep only the names inside that row's filter, which is column 2 of `results.tsv`. Making `board.sh`
do this by itself is follow-up work (see the handoff of item imx8mp-gles3-tree).

### Expected verdict lines

Each boot prints the following. This is the format of imx8mp-plan1.md §1, where the same path ran the
gles2 table:

```
=== e00 (zephyr.bin, selector 0) <YYYY-MM-DD HH:MM:SS>
board.sh imx8mp e00 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08xx,<w1>,<w2>,<w3> back_after=<n>s uptime=<n>s
=== e00 rc=0 <YYYY-MM-DD HH:MM:SS>
```

A DEQP record carries no pass/fail of its own (`status=n/a`), so `verdict=done` means the boot completed
and left a record. It is also `done` for a row with failing cases, so judge a row by its tally, not by this
word. A healthy boot has `boot_verdict=cycled` and `rc=0`. Other outcomes:

| line | meaning |
|---|---|
| `verdict=no_record` | the boot came back without a DEQP record (the image died before its first case) |
| `back_after` near 600 s plus boot, and w3's low half below the entry's mustpass count | a watchdog end: split that entry |
| `rc=5` | the board did not come back, and the chain stops |

## 4. Tally

The `tree` command ends with the tally. To rerun it at any time:

```sh
tools/board.sh imx8mp tally records/imx8mp/es3-$HEAD8
```

It writes `results.tsv` (index, filter, begun, passed, failed, notsup, other, noresult, failing names) and
`rows.md` in the tree directory, prints `e<i>: begun .. pass .. fail .. ns .. other .. no-result ..` for
each row, and ends with:

```
TALLY (133 entries): begun <N> passed <P> failed <F> notsup <S> other <O> noresult <R>
```

How to read it:
- Compare with the AM62P (RESULTS.md §1r): 31,278 pass / 8,835 fail / 2,636 not supported / 5
  QualityWarning, with 56 lifetime cases without a result.
- A row's `begun` should be at least its mustpass count (from the checker's per-entry lines), and it can
  be higher because of non-mustpass cases the wildcards also select. A lower `begun` means the row ended
  early.
- The target for the first run is a complete record for every one of the 133 rows. Zero failures is the
  plan's goal, not this run's.
- To compare two trees, add `TREE_PREV=records/imx8mp/<earlier>` to the `tree` command, or run
  `tools/board.sh imx8mp diff <a> <b>`.

## ES 2.0 regression rows

These are separate and unchanged. Build with the gles2 stage 1 and no `DEQP_MODULE`, then run
`tools/cts/imx8mp-full-tree.txt` (29 rows) as described in `tools/cts/README.md`. Expect 0 failures and rows as
in `tools/cts/imx8mp-full-tree-29460f96.results.tsv`.

```sh
DEQP_STAGE1=<gles2 stage 1> tools/cts/imx8mp-deqp-build.sh wt-$HEAD8 'dEQP-GLES2.info.*' --table tools/cts/imx8mp-full-tree.txt
TREE_IMAGE=$IMX8MP_OUT/wt-$HEAD8/zephyr/zephyr.bin tools/board.sh imx8mp tree tools/cts/imx8mp-full-tree.txt records/imx8mp/es2-$HEAD8
```
