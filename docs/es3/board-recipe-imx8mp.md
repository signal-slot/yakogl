<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# dEQP-GLES3 on the Verdin i.MX8M Plus (GC7000UL): board recipe

This recipe has run on the board. Three slices of it (plan #316 items s01, s03, s04) came from the one
campaign commit `9bc82915e06367b46e3a62b644dbf2bb659d72ad` and are recorded as
`2026-11-imx8mp-gles3-9bc82915-rows01-13`, `-rows17-34` and `-rows35-51` under `tools/cts/log/`: table
entries 0-12, 16-33 and 34-50 (48 of 133, 28,459 mustpass cases). **Entries 13-15 and 51-132 (14,351
mustpass cases) have not run.** Every verdict line, record word and tally line this file expected has been
seen; what has never been seen is the console's text, and the sections that were marked **unverified** now say
what the three sessions measured instead. Board results are recorded under
`tools/cts/log/<date>-imx8mp-gles3-<slice>/`; open work is issue #2 of signal-slot/yakogl. The commands
are the existing gles2 ones plus the gles3 switch.

For a board session read "Running a GLES3 slice session" (after section 4): one ordered procedure over the
sections below.

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

These are estimates, not measurements; measured, they are roomy. Of the 48 recorded entries none ended at
the watchdog and none was re-split: 44 of them ran in 45-105 s from reboot to ssh (Linux's own boot
21-38 s inside that), and the four `shaders.arrays` / `shaders.builtin_functions.precision` /
`shaders.conversions.vector_to_vector` entries took 91-338 s (e27 194 s, e29 91 s, e25 273 s, e26 338 s).
`fbo.blit.conversion` (e4, e5: 69 s and 64 s) and `fragment_ops.blend` (e10-e12) needed no split; the
texture groups (the reason for the 20-150 case budgets) are entries 51-132 and have not run. If an entry
ends at the watchdog, split it: lower that group's budget in `imx8mp-gles3-gen.py`, regenerate the table
and run the checker again. You can tell a watchdog end because the record's `begun` is less than the
entry's mustpass count and `back_after` is near 600 s plus the boot. e26 is the one recorded entry with
cases that never began (123 of its 623) and it is not a watchdog end: it stopped at 338 s with
`verdict=done` and a complete record, the same word for word on a rerun (its slice README, "e26 ends
early"). The 2048x2048 cases can end a run with GL_OUT_OF_MEMORY in the 64 MiB carveout (see the board
conf); no recorded entry reached that.

`lifetime.*` (56 cases) is entry 22 on its own. On the AM62P its init dies, because transform feedback is
missing (RESULTS.md §1r), so isolating it means a crash there costs no other cases. The patterns are
dEQP filters over the whole dEQP tree, so a row can also begin a few cases that are not in the mustpass.

## Environment

The same as for gles2 (`tools/cts/imx8mp-env.sh`): `ZEPHYR_WS`, `ZEPHYR_VENV`,
`ZEPHYR_SDK_INSTALL_DIR` and `IMX8MP_OUT`. You also need `DEQP_STAGE1`. Give gles3 a directory of its own,
because the gles2 directory holds the gles2 libraries and stage 2 links what it finds there. On the board
side, set `BOARD_HOST=<user@host>` from the environment (never write it into a file) and
`BOARD_BOOT=uboot-mmc`. `HAL_NXP_DIR` (the hal_nxp checkout) is not read by the build scripts; the
preflight and `tools/check.sh` check it. `BOARD_UART` (the console) is optional for `board.sh` and required
by the preflight; see "Capturing the console".

## 1. Stage 1 (once per VK-GL-CTS revision, about 1.9 GB)

The VK-GL-CTS checkout is pinned at `93bca01861b0e3ef3c387027a9791e6d065f900c` (the `DEQP_WORK` entry of
`tools/cts/deps.toml`). It needs its external sources too (`external/fetch_sources.py`), and
`cts-build-zephyr.sh` reads the checkout from `CTS=` (its default is a path in your home directory):

```sh
git clone --no-checkout https://github.com/KhronosGroup/VK-GL-CTS <deps>/VK-GL-CTS
git -C <deps>/VK-GL-CTS checkout --detach 93bca01861b0e3ef3c387027a9791e6d065f900c
(cd <deps>/VK-GL-CTS && python3 external/fetch_sources.py)
export ZEPHYR_WS=<west workspace> ZEPHYR_VENV=<workspace>/.venv \
       ZEPHYR_SDK_INSTALL_DIR=<zephyr-sdk-1.0.1> IMX8MP_OUT=<out> CTS=<deps>/VK-GL-CTS
tools/cts/imx8mp-deqp-stage1.sh gles3     # = DEQP_MODULE=gles3 DEQP_STAGE1=<out>/deqp-stage1-gles3
```

The `gles3` argument uses `$DEQP_STAGE1_GLES3` if set, else `$IMX8MP_OUT/deqp-stage1-gles3`. The script also
exports `ZEPHYR_SDK` (from `ZEPHYR_SDK_INSTALL_DIR`) and `-O2` flags, which `cts-build-zephyr.sh` needs.
Measured: 453 ninja steps, about 15 minutes on a 12-thread worker, 2.0 GB. Each of the three slice sessions
rebuilt it in about 5 min on 4 cores (`rc=0`, last step `[453/453] Linking CXX static library
modules/gles3/libdeqp-gles3-package.a`), and the two libraries' shas are the same on every host
(`132944 2fff5184…b2e8`, `12732236 7b842c95…85cc`): the stage 1 is host-portable for one VK-GL-CTS pin. A
fourth build (this file's figures rechecked at `e6e68ff7`, 2026-11-16) took 453 steps and 685 s on a 4-core /
7 GiB worker, `rc=0`, `$IMX8MP_OUT/deqp-stage1-gles3` 2.1 GB — the CTS checkout and its externals are most of
that weight, so the "~1.9 GB" of the heading is the libraries plus the checkout copy. One caveat for a rebuild
after a killed run: `cts-build-zephyr.sh` discards west's output (it never lands in the log), and a `flags/` directory left
half-written by a run killed during the devicetree step makes the next configure die in `gen_defines.py`
(`EOFError: Ran out of input` from `edt.pickle`) with no explanation in the log. Remove
`<stage 1>/flags` (and the log) and run it again; the same command in a clean directory configures in about
17 s.

This exports `MODULE=gles3` to `tools/cts/cts-build-zephyr.sh` (the VK-GL-CTS pin above, with the same
patches as gles2). The log goes to `$IMX8MP_OUT/deqp-stage1.log`, the same file as for gles2.

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
(imx8mp-plan1.md). Measured for the gles3 table image built from commit `6ddba20c` (`es3-6ddba20c`):
`zephyr.bin` 16,443,032 bytes (sha256 `a5213848…4620e4`). The sections of `zephyr.elf` (`zephyr.stat`):
text 11,472,896, rodata 3,521,708 (plus `.gcc_except_table` 402,623 and `.eh_frame` 987,504), data
52,477, bss 42,604,674, noinit 168,858,272. The noinit section holds the libc malloc arena,
`CONFIG_COMMON_LIBC_MALLOC_ARENA_SIZE=167772160` (160 MiB; the image sets
`CONFIG_HEAP_MEM_POOL_SIZE=0`), and 1,086,112 bytes besides.

Built three times at `9bc82915` on three workers, the same figures every time: `zephyr.bin`
`16443032 f9c80e52…4cfc`, `es3-9bc82915.table.conf` `38164 5b7a6123…b82f`, footprint 227909632. The bin is
byte-identical across hosts; only `zephyr.elf` differs per host (its debug paths: 41,869,168, 41,868,592
and 41,869,040 bytes, three different shas). So the elf's sha is that session's, not a campaign constant,
and a slice README quotes its own. One host printed the linker's `Memory region` header with no `RAM:` row:
the linker right-aligns the region names (`             RAM:     233912 KB …`) and the artefacts script's grep
was anchored at the line start, so the indented rows went unmatched; it now allows the indentation, and a
slice README that quoted the header alone quotes the RAM row from the same log. The footprint there is
unchanged, and `board.sh --dry-run` prints `footprint 227909632` whatever the linker printed.

The footprint `board.sh` checks is the AArch64 header's `image_size` (`nb_check_image` in
`tools/uboot-netboot-lib.sh`): Zephyr writes `_flash_used` there, the distance from `0x60000000` to the
end of `.last_section`, which this image links right after `.last_ram_section`. It is 227,909,632 bytes
(222,568 KB, the linker's `RAM` line), 84.9 % of the 256 MiB `a53_ddr_section`; `board.sh` refuses an
image whose `image_size` passes `0x70000000`, and on the `kexec-dev` route one larger than the crash
window. `CONFIG_KERNEL_VM_SIZE=0x20000000` (512 MiB) is twice the region, so it covers the image.

Rebuilt at `e6e68ff7` (2026-11-16, to recheck these figures against current main): stage 1 `rc=0`, 453 steps,
685 s on a 4-core worker, 2.1 GB; image `rc=0` in 165 s — `imx8mp: 133 table rows -> es3-e6e68ff7.table.conf`,
`check-deqp-images: the dEQP image recipes build what conformance needs`, `.config` with
`CONFIG_SERIAL/UART_CONSOLE/STDOUT_CONSOLE/PRINTK =y`, `# CONFIG_DEQP_REPORT_ES2 is not set`,
`# CONFIG_YAKOGL_REPORT_ES2 is not set`, `CONFIG_GC_BOARD_WDOG_SECONDS=600` — `zephyr.bin` 16,642,712 bytes
(sha256 `76fe4a4f…5b28`), `zephyr.elf` 44,557,320 bytes, the table fragment byte-identical to the campaign's,
linker `RAM: 233912 KB 256 MB 89.23%`, dry run `footprint 239525888`.

**The footprint grows with `main`.** Same table, same stage 1, same board conf: 222,568 KB at `9bc82915`,
233,912 KB at `e6e68ff7` — 11,344 KB more in the days between those commits, 84.90 % → 89.23 % of the region.
`board.sh`'s refusal is at `image_size` past `0x70000000` (268,435,456 bytes), so a campaign started at a
current main has about 28.9 MB left. Read the dry run's `footprint` line at the campaign commit before the
first slice, and re-read it whenever an image is rebuilt: the campaign's one commit is what makes the image,
the footprint and the records belong to each other.

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
  touches no board. Done for the built image (`es3-6ddba20c`, sha256 `a5213848…4620e4`): 133 rows
  `e00`..`e132`, selectors 0..132, `footprint 227909632`, and the same for each slice on its own image
  (`e00`..`e12`, `e16`..`e33`, `e34`..`e50`, selectors equal to the entry index, `footprint 227909632`).
- **Subset:** `TREE_ROWS=0-9` or `TREE_ROWS=57,60-66` runs only those rows.
- **Resume:** a row whose run directory already has an `INDEX.txt` is skipped.
- **`RUN_BOUND=800`** (what the three slices used). The imx8mp default is `RUN_BOUND=600`, the WDOG1 window
  itself, so a row that ran its window out would read as rc 5 before the board was back: 800 is the AM62P
  dEQP rule (600 + 60 + boot). No recorded row came near either bound (longest 338 s).
- **`BOARD_MUSTPASS=tools/cts/gles3-mustpass.txt`**: `board.sh`'s console tally and diff default to the
  gles2 list; a GLES3 run sets it to the gles3 list. It does not change the CRC name lookup, which reads
  both mustpass files (next section).

The three slices ran it as two passes over one directory: `NAMES=0` over the rows first, then the same
command with `NAMES` unset to run the name pages (see "Failing names").

Row *i* is run `e<i>` (`e00` .. `e132`) with SNVS selector *i*. The image reads the selector from the
SNVS (`gc_result.h`), runs table entry *i*, and resets through WDOG1 when it is done or after 600 s.

- A refusal (rc 3) is retried 60 times, 30 s apart.
- A board that does not come back (rc 5) stops the chain.
- Budget: 133 boots of about 50-700 s each, so several hours.

### How records are read

The image's own result channel is the SNVS record (the console is a second, separate channel; see
"Capturing the console" below). After every case the image rewrites its four-word SNVS DEQP record. After the
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
`NAMES=0` turns this off. Measured: a page boot runs the whole entry again (`main.cpp` masks the selector
`& 0xff`) before recording its six CRCs, so a page costs as much board time as the row does: 44-299 s in
the three slices (76 pages for 13 rows, 74 for 18, 7 for 17). Every page of a row carries the row's own
fail count in w0's low byte, and the failure sets were stable across all 157 reruns. `board.sh` reads at
most 40 pages (240 names) of a row: e25 (364 fails), e26 (277 fails + 1 other) and e29 (328 fails) were
left with counts and no names, because naming them would have named only part of the row and cost about
8 h of board time. Raise the limit, split the entry, or get a working console (next section) to name such
rows.

**Failing names.** `board.sh` maps each CRC to a name with `yakogl-tools record crc <c>`. Its case list
is `tools/cts/gles2-mustpass.txt` and `tools/cts/gles3-mustpass.txt` together and it matches a check
against every name in it, so a GLES3 table image's failures resolve as they are — kept only when they start
inside the row's own table entry (column 2). To search one module or another list only: `record crc <hex>
dEQP-GLES3.`, or `DEQP_CASELIST=<file>` to replace the list.

What the three slices measured, against what this section expected:

- **`crc:<hex>` does appear in `failing names`.** A CRC whose only resolutions lie outside the row's
  filter, or which resolves to no dEQP-GLES3 name at all, is printed as a CRC: rows01-13 e8 `crc:118d`,
  `crc:23fb`, `crc:579a`; rows17-34 e22 1, e23 1, e24 2, e27 14 such CRCs. They are the cases a row's
  wildcards also select outside the mustpass list (e8 +11, e22 +2, e23 +17, e24 +8, e27 +462 begun over
  mustpass).
- **A name check of two names can be equal and both are printed**, joined by `,` in one column: e9's CRC
  `ec9b` (`fbo.msaa.2_samples.rgb5_a1` and `fbo.render.recreate_color.tex2d_rg8ui_depth_stencil_rbo_depth24_stencil8`),
  e2's CRC `f305`, e19 1 pair and e27 2 pairs. e9's pair appears **twice** in the list, because two pages
  recorded the same CRC at two positions of the run (e09-p1, inside `fbo.msaa.*`, and e09-p9, inside
  `fbo.render.*`): the position in the list says which case it is, both cases fail. A row whose names
  exceed its fail count is made of these pairs, not of invented cases (e9: 114 names for 114 fails, 112
  single names plus the pair twice).
- **`merge.py --check` calls such a pair `case listed twice`** and fails the campaign over it (step 13).
  The list is left as `board.sh tally` wrote it; the page positions in the slice README resolve the pair.
- e2's 24 fails show how a collision is settled from the pattern: they are exactly the six non-square
  matrix types of `default_vertex_attrib` in all four variants (6 x 4), so the pair is
  `mat4x2.vertex_attrib_4fv` and `mat4.vertex_attrib_4f` passes.

### Capturing the console

The Mallow carrier has an A53 console: Verdin UART_3 (i.MX8MP UART3, Linux `ttymxc2`), reached through a
serial bridge or a local device whose address comes from the environment (`BOARD_UART`; it is never written
into a file or a commit). What the code does:

- **What the sample prints.** When a console is attached, the deqp sample (`deqp_log.c`) prints
  `deqp: <Status> <case name>` per case, `deqp: selector <n>`, `deqp: done: ...` and
  `deqp: summary: N cases begun, M with a result, K without` at the end (`main.cpp`).
  `tools/cts/deqp-uart-tally.py` reads that format.
- **The image sends it only when built for it.** `boards/verdin_imx8mp_mimx8ml8_a53.conf` sets
  `CONFIG_UART_CONSOLE=n`, so an image built without the switch prints nothing on UART3.
  `IMX8MP_CONSOLE=1 tools/cts/imx8mp-deqp-build.sh ...` adds the fragment `tools/cts/imx8mp-console.conf`
  (`CONFIG_SERIAL`, `CONSOLE`, `UART_CONSOLE`, `PRINTK`, `STDOUT_CONSOLE` = y; the board devicetree already
  has `zephyr,console = &uart3`) after the table fragment. Only the value `1` counts. Zephyr accepts the
  fragment: every slice's `.config` has `CONFIG_UART_CONSOLE=y`, `CONFIG_STDOUT_CONSOLE=y` and
  `CONFIG_PRINTK=y` (checked with `grep -E '^CONFIG_(UART_CONSOLE|PRINTK|STDOUT_CONSOLE)=y'` and reported by
  `imx8mp-gles3-artefacts.sh`'s `# CONFIG_DEQP_REPORT_ES2 is not set` / `# CONFIG_YAKOGL_REPORT_ES2 is not
  set` lines). What it does not yet give is readable output: see "What the three sessions measured" below.
- **What `board.sh` does.** With `BOARD_UART` set, an imx8mp `tree`/`run` on `uboot-tftp` or `uboot-mmc`
  probes the bridge (refusing when it is unreachable or busy) and captures the console into each run
  directory: a `host:port` bridge into `console.raw`, `console.ts` and the derived `console.txt`; a local
  device line by line into `console.txt` (`HH:MM:SS line`). With `BOARD_UART` unset nothing is captured and
  the run reads only the SNVS record, as before. A bridge takes one client at a time, so nothing else may
  hold it during a run.
- **Reading a capture.** `tools/cts/imx8mp-console-scan.py` lists GPU recoveries, watchdog stops and each
  row's board time; `board.sh` itself does not count GPU recoveries on this board (its `resets=` stays
  `absent`). See the procedure, step 10.
- **Names from a console instead of CRC paging.** The failing names are the `deqp: <Status> <case>` lines
  of the run's `console.txt` (`tools/cts/deqp-uart-tally.py` tallies them). Then `NAMES=0` can switch off the
  `e<i>-p<page>` boots (`board.sh` runs them only when `NAMES` is not `0`; each failing row costs an extra
  boot of the same table image per page of six CRCs). The tally still comes from the SNVS record. Lost
  console lines make a log incomplete (`board.sh` then says `done_lossy`, the tally script INCOMPLETE/MISSING);
  keep CRC paging for any row whose console log is not complete.
**What the three sessions measured.** Each was built with `IMX8MP_CONSOLE=1`, each had the three `.config`
lines above, and `board.sh` captured `BOARD_UART` for every boot (`console.raw`, `console.ts`,
`console.txt` in each run directory): the leased console returned **no text at all**. One NUL byte
(`\x00`) at each board reset and nothing else — 89 bytes for the 89 boots of rows01-13, 78 for the first 78
boots of rows17-34. The adapter reported itself `connected` (115200 8N1, its own settings) the whole time.
Before the runs, a marker line written to the board's UART3 device from Linux (as root, several times) did
not arrive, and a bare Enter got no login prompt back although `agetty` / `serial-getty` was active on that
UART at 115200. `BOARD_UART` named a local feed bridge (`tools/am62p_netboot_uart.py feed`, the route for a
console the farm hands out as MCP tools only): whether a client outside the farm still holds the bridge's
data, or the UART_3 wiring is open, cannot be told from the host. So the capture is complete in the sense
that `board.sh` wrote what the bridge returned for every boot, and the run is not disturbed by it — but
there is nothing in it. What follows for every session so far:

- `imx8mp-console-scan.py` sees 0 boot banners and 0 rows in every capture (at most 1 line: the time stamp
  of the NUL), so **GPU recoveries are not observable**: `recoveries 0` is not shown for any row, and the
  zero-recovery rule of the session rules cannot be *proved* from a scan yet. A slice reports that as
  found (both slices' READMEs do), and rests the row's health on `rc=0 verdict=done`, `resets=absent`, a
  complete record and `meta.txt`'s boot-boundary checks (`uptime` below the time since the reboot).
- No board time comes from the scan. The per-row times are `back_after` and `uptime`, from `meta.txt`
  (`t_reboot`, `t_back`, `up`).
- Statuses other than Fail (`NotSupported`, `other`) have no names at all: the record carries counts, so
  `merge.py --notpass` lists them as unresolved. The failing names come from the CRC pages, at one rerun of
  the whole entry per six names.
- `tools/cts/deqp-uart-tally.py`, `done_lossy`, and the "names from a console instead of CRC paging" route
  above are unusable on this board until the console delivers bytes. Until then: `NAMES=0` for the rows,
  then the pages, and `BOARD_MUSTPASS=tools/cts/gles3-mustpass.txt`.

### Expected verdict lines

Each boot prints the following. This is the format of imx8mp-plan1.md §1, where the same path ran the
gles2 table:

```
=== e00 (zephyr.bin, selector 0) <YYYY-MM-DD HH:MM:SS>
board.sh imx8mp e00 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08xx,<w1>,<w2>,<w3> back_after=<n>s uptime=<n>s
=== e00 rc=0 <YYYY-MM-DD HH:MM:SS>
```

Seen for every boot of the three slices, this shape exactly (`e00` .. `e50`, and each slice's README quotes
its rows verbatim). A name page prints the same three lines with `e09-p1` where the row has `e09`, the page
selector (`0x8009` here), `result=record:DEQP_FAILS` and a w0 of kind `0a`; the slices keep every one of
them in `verdicts.txt`, the rows and the pages together.

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
- Compare with the AM62P whole tree (RESULTS.md, "ES 3.0: dEQP-GLES3": 36,552 pass / 2,880 fail / 2,195
  NotSupported / 185 QualityWarning / 998 without a result, on `ad2b5ed`). The 48 recorded i.MX8MP entries
  cover 28,459 mustpass cases and read **27,201 pass / 1,851 fail / 530 NotSupported / 12 other / 123
  without a result** (begun 29,594, which counts the cases the wildcards also select).
- A row's `begun` should be at least its mustpass count (from the checker's per-entry lines), and it can
  be higher because of non-mustpass cases the wildcards also select. A lower `begun` means the row ended
  early. Measured: a low `begun` is not by itself a watchdog end — e26 began 500 of its 623 in 338 s, with
  `verdict=done` and a full record, and its rerun gave the same record (its slice README). Compare
  `back_after` with the window before calling it a watchdog end.
- The target for the first run is a complete record for every one of the 133 rows: 48 have one, 85 do not.
  Zero failures is the plan's goal, not this run's.
- To compare two trees, add `TREE_PREV=records/imx8mp/<earlier>` to the `tree` command, or run
  `tools/board.sh imx8mp diff <a> <b>`.

## Running a GLES3 slice session

One ordered procedure for a board session that runs a row range of the 133-entry table and commits it as a
slice. Every command below exists in the tree with the flags shown, and the output described is what its script
prints (read from the script). Steps 1-13 have been run three times on the board, from the one commit
`9bc82915`, as `2026-11-imx8mp-gles3-9bc82915-rows01-13`, `-rows17-34` and `-rows35-51`: what the board
prints is what those runs recorded. The open board-side unknown is the console's **content** (the bridge
delivered no text in all three sessions), not the verdict lines, the records or the tallies.

Rules that hold for every step:
- **One commit.** All slices of a campaign come from the same `origin/main` commit, and the image is named after
  it (`es3-<hash8>`). `tools/cts/imx8mp-gles3-merge.py --check` fails when two slices name different commits.
  Check the commit before each slice (steps 1 and 7).
- **One GL client on the board at a time.** `board.sh` takes the board lock and the board lease
  (`/var/tmp/yakogl-board.lease` on the board, waited for up to `LOCK_WAIT`) and refuses (rc 3, retried 60 times,
  30 s apart on the imx8mp) when another session, a console or GPU client or a pending one-shot exists. Start no
  second session, no Linux GL program and no second console reader on the board or its bridge meanwhile.
- **Never brick the board.** Every image arms WDOG1 (`gc_wdog_arm: armed`; `board.sh` refuses an image without
  that marker) and feeds it for a bounded time, 600 s (`CONFIG_GC_BOARD_WDOG_SECONDS=600`), then the SoC resets.
  Run only the table image, through `tree`. Do not use `board.sh imx8mp default ... forever`, `kexec-dev` or
  hand-written U-Boot commands. A row that ends rc 5 (`the board did not come back`) stops the chain: stop the
  session and report; do not keep booting.
- **Zero resets.** A slice counts only when every row is `verdict=done` with `resets=absent` or `0` and the
  console scan shows `recoveries 0` for it. `board.sh` does not count GPU recoveries on this board (its
  `resets=` stays `absent`), so the scan (step 10) is the only recovery check. A row with recoveries is recorded
  as it is and rerun; it is not a clean row. Measured: the scan has shown `recoveries 0` for no row, because it
  has had nothing to read (see "What the three sessions measured"). All three slices are recorded with that
  proof missing, and say so in their READMEs. Do not write `recoveries 0` from a capture without `deqp:` lines:
  until the console delivers bytes, what a clean row rests on is `rc=0 verdict=done`, `resets=absent`, a complete
  record with counts that fit the entry, and `meta.txt`'s boot boundaries (`uptime` below the time since the
  reboot, the board's uptime matching the last return).
- **Board times.** Record each row's time: `back_after=<n>s` of its verdict line and the scan's elapsed time.
- **No addresses in files.** `BOARD_HOST` and `BOARD_UART` come from the environment only. Name them in a
  README, never their values. Before committing, read every file for `@` logins, dotted-quad numbers, `:port`
  and `/dev/`.

1. **Fix the commit.**
   ```sh
   git fetch origin
   COMMIT=$(git rev-parse origin/main); HEAD8=${COMMIT:0:8}
   git rev-parse HEAD         # must equal $COMMIT when you build: the image is built from this checkout
   ```
   The first slice sets `COMMIT` for the campaign. Later sessions use the same value even if `origin/main` has
   moved (pass it as `--commit` in step 7). A new campaign is needed only when the table or the image code
   changed. If `git fetch` fails, say so; do not take a stale ref for the current one.

2. **Environment** (names only; the values come from the shell environment): `IMX8MP_OUT`, `DEQP_STAGE1`
   (the gles3 stage 1 directory; `$IMX8MP_OUT/deqp-stage1-gles3` is the default of the `gles3` arguments),
   `ZEPHYR_WS`, `ZEPHYR_VENV`, `ZEPHYR_SDK_INSTALL_DIR`, `HAL_NXP_DIR`, `BOARD_HOST`, `BOARD_BOOT=uboot-mmc` and
   `BOARD_UART`. `DEQP_STAGE1` has to name the **gles3** stage 1 in the shell too: `imx8mp-deqp-build.sh
   --gles3` and `imx8mp-deqp-stage1.sh gles3` set what they build from (`$DEQP_STAGE1_GLES3`, default
   `$IMX8MP_OUT/deqp-stage1-gles3`), but the preflight (step 7) and step 3 read `DEQP_STAGE1` and look in it
   for `build/modules/gles3/libdeqp-gles3-package.a` — with `DEQP_STAGE1` naming a gles2 stage 1 the preflight
   prints `FAIL stage 1 package missing (<DEQP_STAGE1>/build/modules/gles3/libdeqp-gles3-package.a)` and every
   other line stays ok, which reads like a broken image rather than a wrong variable. `tools/cts/imx8mp-env.sh` is sourced by the build scripts, not run. It needs `ZEPHYR_WS` (the
   old `ZEPHYR_WORKSPACE` is still read), `ZEPHYR_VENV`, `ZEPHYR_SDK_INSTALL_DIR` and `IMX8MP_OUT`, creates
   `$IMX8MP_OUT`, and exits 2 with `imx8mp: <NAME> is not set` (or `... not found`) when one is missing.

3. **Stage 1** (skip it when `$DEQP_STAGE1/build/modules/gles3/libdeqp-gles3-package.a` exists for the pinned
   VK-GL-CTS revision; section 1 has the checkout):
   ```sh
   tools/cts/imx8mp-deqp-stage1.sh gles3
   ```
   Prints `rc=<n>` and the last 5 log lines (log: `$IMX8MP_OUT/deqp-stage1.log`); on a failure it also prints the
   first `FAILED:` block. The exit status is the build's (2 for a bad `DEQP_MODULE`). Expected: `rc=0`.

4. **Table image**, console on:
   ```sh
   IMX8MP_CONSOLE=1 tools/cts/imx8mp-deqp-build.sh --gles3 es3-$HEAD8 'dEQP-GLES3.info.*' \
       --table tools/cts/imx8mp-gles3-tree.txt
   ```
   Only the value `1` of `IMX8MP_CONSOLE` adds the console fragment; without it the image is silent and steps
   10 and 11 have no console to read. Build one kind of image per campaign under the one name. Expected output:
   `imx8mp: 133 table rows -> <out>/es3-<hash8>.table.conf`, `rc=0`, then
   `check-deqp-images: the dEQP image recipes build what conformance needs`. A `check-deqp-images: FAIL ...`
   line followed by `imx8mp: warning: not a conformance image` means the image would not report ES 3.0. The
   exit status is the build's; a usage error exits 2. The log is `$IMX8MP_OUT/es3-$HEAD8.log`. Check that the
   console took:
   `grep -E '^CONFIG_(UART_CONSOLE|PRINTK|STDOUT_CONSOLE)=y' $IMX8MP_OUT/es3-$HEAD8/zephyr/.config` should print
   three lines (measured: each of the three builds printed all three; if one is missing, stop and report). The
   build also stamps itself (`driver/samples/zephyr/common/sample.cmake`): the configure log prints `pvr
   sample build: tree <git describe --always --dirty of the build checkout> table <sha256 prefix of
   tests/reference/blobs.rs>` — the three slices read `tree 9bc82915 table 6c5b8a70`, zephyr
   `af17c0c8aa2a`. `tree` is the campaign commit (its `dirty` suffix is how a build of a worktree with an
   uncommitted `blobs.rs` shows itself); `table` is the **shader** table's hash, not the case table's. The
   image prints it on the console before its first case (`sample_glue.c`); the dry run's per-row line carries it
   (`build <tree>`) and each run's `meta.txt` records it as `image_build=`. The `dirty` suffix works: a dry run
   of an image built in a checkout with uncommitted edits read `build e6e68ff7-dirty`.

5. **Artefacts for the session note:**
   ```sh
   tools/cts/imx8mp-gles3-artefacts.sh es3-$HEAD8
   ```
   Prints `<bytes> <sha256> <path>` for `zephyr.bin`, `zephyr.elf`, `es3-$HEAD8.table.conf`,
   `libdeqp-gles3-package.a` and `libdeqp-gles3-functional.a`, then the two `.config` lines for
   `CONFIG_DEQP_REPORT_ES2` and `CONFIG_YAKOGL_REPORT_ES2` (`# ... is not set` expected; `... not in .config` if
   absent) and the linker's `Memory region`/`RAM:` lines. It exits non-zero when a file is missing. These lines
   go into the slice README. Measured across the three slices and a fourth build at `e6e68ff7`: the stage 1
   shas are the same on every host and on every commit that keeps the VK-GL-CTS pin (`2fff5184…`, `7b842c95…`);
   `zephyr.bin` and `es3-<hash8>.table.conf` are byte-identical across hosts for one commit; `zephyr.elf` is not
   (its debug paths), so a README quotes the elf sha of its own build. `imx8mp-gles3-artefacts.sh` matches the
   linker's indented `Memory region`/`RAM:`/`FLASH:` rows (the linker right-aligns the region names; earlier
   versions anchored the grep at the line start and printed the header with no `RAM:` row, seen in slice
   rows17-34). The dry run's `footprint <bytes>` line is the fallback when the build log is absent.

6. **Plan the slices** (host only):
   ```sh
   python3 -I tools/cts/imx8mp-gles3-sessions.py --sessions 6 --overhead 60     # or --session-budget <seconds>
   python3 -I tools/cts/imx8mp-gles3-sessions.py | awk -F'\t' '$5=="RISK"'      # entries near the window
   ```
   With neither option it prints a `#` header (every rate and its source) and one row per entry: `index cases
   est_seconds rate_source risk first_pattern`. With `--sessions N` or `--session-budget S` it prints `session
   start end entries cases est_seconds risky_entries`: contiguous inclusive ranges of entries counted from 0,
   which is what `TREE_ROWS` takes. `risk` is `RISK` when the estimate passes 0.8 x 600 s (`--margin`,
   `--window`, `--scale` change it). The rates are the Pi 4 record's where measured and otherwise assumed from
   the table header; the GC7000UL/V3D ratio is unmeasured, so the figures order the work, they are not limits.
   Exit 1 with `WARNING:` on stderr when the table and the mustpass list disagree. The host pre-triage expects
   hangs on entries 13-15, 57, 95 and 122-127 and watchdog ends on 58-60: keep those out of the first slices
   and run them last, in small ranges. Measured, none of the 48 recorded entries hung or ran out its window,
   and the estimates were high rather than low for them (`rows35-51` ran 45-105 s an entry where the planner
   said 106-298 s). None of the RISK-flagged entries has run, so those expectations are still open: run a small
   range of them first (`TREE_ROWS=13-15` is the nearest unrun one: 450 + 250 + 483 = 1,183 mustpass cases,
   the rest of `fragment_ops.blend.fbo_srgb`).

7. **Preflight** (read-only; no network, no board):
   ```sh
   tools/cts/imx8mp-gles3-preflight.sh --commit "$COMMIT" --out "$IMX8MP_OUT"      # also: --table <file>
   ```
   One line per check, `  ok   <text>` or `  FAIL <text>`: `IMX8MP_OUT`, `DEQP_STAGE1`, `ZEPHYR_WS` and
   `HAL_NXP_DIR` set and existing, `BOARD_UART is set`, the stage 1 package, `table passes
   check-imx8mp-gles3-tree.py`, `table image exists (es3-<hash8> ...)`, `image es3-<hash8> is commit <hash8>`.
   The last line is `preflight: all ok` (exit 0) or `preflight: <n> check(s) FAILED` (exit 1); a bad option exits
   2. `--commit` defaults to `git rev-parse origin/main`, so pass the campaign commit once main has moved. Run
   it before every slice. Run against the real build three times it printed nine `ok` lines and
   `preflight: all ok` (its selftest covers stub files only). `DEQP_STAGE1` must name the gles3 stage 1 (step
   2): the `stage 1 package` line is the only one that reads the gles3 path, so a gles2 `DEQP_STAGE1` gives two
   `FAIL` lines (`DEQP_STAGE1 is set, directory does not exist` and `stage 1 package missing …`) and seven `ok`,
   which reads like a broken image rather than a wrong variable. In a checkout where `origin/main` does not
   resolve (a detached clone, a partial one) it adds `FAIL no commit: origin/main does not resolve (give
   --commit)`: pass `--commit` explicitly there.

8. **Dry run** the slice (touches no board):
   ```sh
   TREE_ROWS=0-9 TREE_IMAGE=$IMX8MP_OUT/es3-$HEAD8/zephyr/zephyr.bin \
     tools/cts/imx8mp-gles3-board.sh --dry-run tree records/imx8mp/es3-$HEAD8-rows01-10
   ```
   Prints the `DRY host$ ...` steps for each selected row (`e00`, ..., selector = entry index) and checks the
   image on the host. `TREE_ROWS` counts entries from 0, inclusive (`0-9`, `57,60-66`). Always set `TREE_IMAGE`:
   without it the wrapper takes the newest `$IMX8MP_OUT/es3-*/zephyr/zephyr.bin` by modification time, which can be
   another commit's image. The wrapper only fills in the image and `tools/cts/imx8mp-gles3-tree.txt`; its one
   option is `--dry-run`, its verbs are `tree` and `tally`, and anything else exits 2.

9. **Board state** (reads the board; `BOARD_HOST` and `BOARD_BOOT=uboot-mmc` set):
   ```sh
   tools/board.sh imx8mp status
   ```
   Shows the lease (`none` expected), the board clock and the eMMC staging area. A foreign lease or session
   means stop. If the real run is refused because the U-Boot hook is not installed, `tools/board.sh imx8mp
   install` is the one-time fix, by whoever owns the board; do not improvise.

10. **Run the slice, then scan the console.**
    ```sh
    OUT=records/imx8mp/es3-$HEAD8-rows01-10
    TREE_ROWS=0-9 TREE_IMAGE=$IMX8MP_OUT/es3-$HEAD8/zephyr/zephyr.bin BOARD_BOOT=uboot-mmc \
      BOARD_MUSTPASS=tools/cts/gles3-mustpass.txt RUN_BOUND=800 BOARD_UART=<local feed bridge> \
      NAMES=0 tools/cts/imx8mp-gles3-board.sh tree "$OUT" 2>&1 | tee "$OUT.log"      # pass 1: the rows
    TREE_ROWS=0-9 TREE_IMAGE=$IMX8MP_OUT/es3-$HEAD8/zephyr/zephyr.bin BOARD_BOOT=uboot-mmc \
      BOARD_MUSTPASS=tools/cts/gles3-mustpass.txt RUN_BOUND=800 BOARD_UART=<local feed bridge> \
      tools/cts/imx8mp-gles3-board.sh tree "$OUT" 2>&1 | tee -a "$OUT.log"           # pass 2: the name pages
    ```
    The two passes are what the three slices did: `NAMES=0` keeps the pages out of the first pass (a page is a
    full rerun of its entry), then the same command without `NAMES` runs the pages for the rows that have
    failures or other statuses. Rows whose fails exceed 40 pages can be left for a later session (see "How
    records are read"). A long pass 2 can be split by `TREE_ROWS` (slice `rows17-34` ran it as three passes:
    `16-33`, then `23,24,27`, then `27`); stop a pass **before** arming the next boot, then `board.sh imx8mp
    status` and clear the unfinished run directory (it holds only `meta.txt` and an empty capture, no
    `INDEX.txt`) out of `$OUT` before the next pass.
    Wait for `TREE-EXIT`. Per row it prints the three lines of "Expected verdict lines" (`=== e00 ...`,
    `board.sh imx8mp e00 rc=0 verdict=done ... resets=absent record=594b08xx,... back_after=<n>s uptime=<n>s`,
    `=== e00 rc=0 ...`). A row with failures, other statuses or cases without a result then boots again as
    `e00-p0`, ... unless `NAMES=0`. At the end come the tally (`TALLY (133 entries): ...`; it counts every
    table line, and the rows not run show `-`) and `TREE-EXIT`. A row's run directory
    `$OUT/<YYYYMMDD-HHMM>-e00-<sha8>/` holds `meta.txt`, `INDEX.txt`, `snvs.txt` and, with `BOARD_UART` set,
    `console.raw`, `console.ts` and `console.txt`. `tree` goes on after a row that is not `done` (rc 4 for that
    row) and stops at rc 5. Running the same command again skips every row that has an `INDEX.txt`: to redo a
    row, move its run directories (`e<NN>` and `e<NN>-p<page>`) out of `$OUT` first and say so in the README.
    Scan the capture:
    ```sh
    python3 -I tools/cts/imx8mp-console-scan.py --watch 13-15,57,58-60,95,122-127 \
      $(ls -d "$OUT"/*/ | grep -v -e '-p[0-9]\+-[0-9a-f]\{8\}/$')
    ```
    The arguments are files or run directories (a directory means its `console.txt`, else `console.ts`, else
    `console.raw`); `-q` prints only the per-row table; `--one-based` reads `--watch` as table line numbers. It
    prints `== <file> (<n> lines, <b> boot banner(s), <r> row(s))`, the events by kind (`ROW` selector line, `REC`
    GPU recovery, `WDOG` watchdog arming and end and every boot banner, `FAULT`, `END`) and one line per row:
    `row <n> <start> <elapsed> results <n> recoveries <n> finish deqp: summary: ...`. A row that stopped is
    `!! NO FINISH row <n> (hang | watchdog end | reset)` with the last case that printed a result. Exit 0: every
    row finished; 1: a row did not; 2: bad input. A row is clean with a `finish` line, `recoveries 0` and an
    elapsed time well under 600 s. A capture without `deqp:` lines means the image was built without
    `IMX8MP_CONSOLE=1` or the bridge missed the run (**unverified** on this board).

11. **Read the slice's verdicts** (no board):
    ```sh
    tools/cts/imx8mp-gles3-board.sh tally "$OUT" | tee "$OUT.tally.txt"
    ```
    It recomputes every run's verdict line from its files, then writes `results.tsv` and `rows.md` in `$OUT`.
    Every `e<NN>` row of the slice must read `rc=0 verdict=done` and `resets=absent` or `0`. The others:
    `verdict=no_record` (died before the first case), `done_lossy` (the capture dropped result lines: rerun),
    `done_stopped` (dEQP ended the session early, `complete=0`: the cases after the last one never began),
    `done_with_resets`, `died_after=<stage>`, or `back_after` near 600 s plus boot with `begun` below the entry's
    mustpass count (a watchdog end: step 16). Rerun a row that is not clean once; if it repeats, record it as it is
    and treat it in step 16 or the triage. Note each row's `back_after` and scan elapsed time. Measured: all 48
    rows and all 157 page boots read `rc=0 verdict=done` with `resets=absent`; no row read `no_record`,
    `done_lossy`, `done_stopped` or `died_after` on this board. e26 is the shape to know: its record shows the
    shortfall (`begun` 500 of 623 mustpass) while the row still reads `done`, and its rerun (its own directory,
    `NAMES=0`) gave the same record word for word — the shortfall was recorded as it is and sent to the triage,
    not fixed by a split (step 16 does not help when the stop is a property of one case).

12. **Commit the slice** as `tools/cts/log/<YYYY-MM-DD>-imx8mp-gles3-<hash8>-rowsAA-BB/` (`tools/cts/log/README.md`:
    the board clock's day; `rowsAA-BB` counts table lines from 1, so `TREE_ROWS=0-9` is `rows01-10`). Contents:
    - `results.tsv` and `rows.md` from `$OUT`;
    - `README.md`: the **full 40-hex** `origin/main` commit, the step 5 artefact lines, the exact commands with the
      knob names (`TREE_ROWS`, `NAMES`, `IMX8MP_CONSOLE`, `BOARD_BOOT`; never the values of `BOARD_HOST` or
      `BOARD_UART`), the session window, the verdict lines verbatim from step 11 (`board.sh imx8mp e<NN> rc=0
      verdict=done ... resets=...`; merge reads exactly these lines of the README or of an `INDEX.txt` and fails a
      slice without them), per-row `back_after` and scan elapsed time, recoveries per row and the evidence that the
      board did not reboot outside the rows;
    - each row's `console.txt` as `e<NN>.console.txt` (merge reads `*console.txt` at the top and
      `<dir>/console.txt` below it; a committed `.txt` starts with the three `#` lines of
      `tools/cts/log/README.md`) or a `cases.tsv`, and the scan output as `console-scan.txt`.
    `console.raw` and `console.ts` stay out. `board.sh` redacts the copies in `$OUT`; read every file again.

    What the three slices on `origin/main` look like: `2026-11-imx8mp-gles3-9bc82915-rows01-13`, `-rows17-34`,
    `-rows35-51` — `<YYYY-MM>`, the name shape the Pi 4 gles3 slices of the same shape use, while
    `tools/cts/log/README.md`'s rule asks for the run day. Each holds `README.md`, `results.tsv` and `rows.md`
    with their REUSE `.license` sidecars (the tool's output carries no header of its own), `verdicts.txt` (every
    row and page verdict line of the run log), one `e<NN>.console.txt` per row and `console-scan.txt`. The
    README's verdict lines are step 11's lines in a fenced block, verbatim, which is what `merge.py` reads; the
    per-entry numbers go in a table beside `merge.py --totals`'s rows (`back_after`, `uptime` at the return, page
    count), with a "Resets and reboots outside the rows" section quoting `meta.txt` (the board's uptime at the
    end of the session against the last boot's `t_back` minus its `uptime`), and a "Console" section saying what
    the capture held.

13. **Merge check** of one slice, then of all of them:
    ```sh
    python3 -I tools/cts/imx8mp-gles3-merge.py --check --partial tools/cts/log/<slice dir>
    python3 -I tools/cts/imx8mp-gles3-merge.py --check tools/cts/log/*-imx8mp-gles3-<hash8>-rows*
    ```
    Prints `FAIL: <problem>` lines and `checked <n> inputs, <m> entries with a record: OK` (exit 0) or `FAILED`
    (exit 1). It fails on an entry in two slices, different commits, a row that does not fit the table, a case
    listed twice, a slice without verdict lines, an entry whose verdict is not `done` with rc 0 and resets
    absent or 0, and (without `--partial`) entries with no record: `no record: <n> of 133 entries (<span>), <k>
    mustpass cases`. `--table` and `--mustpass` default to the committed files.

    Measured on the three committed slices: `--check --partial` is OK on `rows17-34` and `rows35-51`, and
    `rows01-13` fails even with `--partial`:

    ```
    FAIL: case listed twice: dEQP-GLES3.functional.fbo.msaa.2_samples.rgb5_a1 (e9)
    FAIL: case listed twice: dEQP-GLES3.functional.fbo.render.recreate_color.tex2d_rg8ui_depth_stencil_rbo_depth24_stencil8 (e9)
    FAIL: no record: 85 of 133 entries (13-15,51-132), 14351 mustpass cases
    checked 3 inputs, 133 entries with a record: FAILED
    ```

    The `no record` line is what a campaign looks like mid-way and is expected until every entry has run. The
    `case listed twice` pair is e9's CRC-16 collision ("Failing names"): the merge reads a name as a case, so a
    name printed twice for one case fails the check, and no run of the campaign passes while that
    `results.tsv` stands. `rows01-13`'s README carries the resolution (the page positions, the 114 names for 114
    fails); a tool that resolved a repeated name by its page would let the campaign close. Until then, run the
    check per slice, quote the `FAIL` line in the report, and do not rewrite `results.tsv` to make it pass.

14. **Not-pass list, totals and triage** (the check of step 13 read as the campaign's state: its `no record` line
    while entries are unrun, its `case listed twice` line while a collision stands):
    ```sh
    D=$(ls -d tools/cts/log/*-imx8mp-gles3-<hash8>-rows*)
    python3 -I tools/cts/imx8mp-gles3-merge.py --totals $D
    (printf '# SPDX-License-Identifier: Apache-2.0\n# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.\n'
     python3 -I tools/cts/imx8mp-gles3-merge.py --notpass $D) > docs/es3/imx8mp-gles3-notpass.tsv
    python3 -I tools/cts/gles3-triage.py docs/es3/imx8mp-gles3-notpass.tsv \
      --host docs/es3/host-gles3-tip.tsv --table tools/cts/imx8mp-gles3-tree.txt > issue-drafts.md
    ```
    `--totals` prints a per-entry table (`mustpass begun pass fail n.s. other none`), the `total` row and
    `no record: ...`. `--notpass` prints `#` header lines (commit, entries run) and one `e<i>:<Status> <case>`
    per case that is not Pass: Fail, NotSupported, QualityWarning, CompatibilityWarning, ResourceError,
    InternalError, Crash, Timeout, and `NoResult` for every case of an entry without a usable record. It adds `#
    e<i>: unresolved ...` comments for cases nothing places. Failing names come from `results.tsv`; every other
    status comes from the slice's consoles or `cases.tsv`, so a slice without them lists only the Fails. In
    these two modes the problems that `--check` fails on are warnings on stderr. `gles3-triage.py` writes
    markdown issue drafts to stdout: cases grouped by their path cut at `--depth` (4) levels, with the host
    verdict of the group (`host-also-fails`, `board-only`, `host-mixed`, `host-notsupported`, `host-unknown`;
    `--gpu` defaults to `vivante`, `--examples` is 5) and the table entry `eNN`. Only the host's non-render groups
    are a firm guide. Agents never run `gh`: the drafts go into the report. Keep `issue-drafts.md` out of the
    commit.

    Two things measured on the campaign (`9bc82915`, three slices):

    - **Filter the not-pass list to the entries that have a record before triaging.** `--notpass` writes
      `NoResult` for every case of every entry without one, and `--notpass` over a subset of slices still counts
      the whole 133-entry table: 15,333 rows for the three committed slices, of which 14,351 are the 85 unrun
      entries' `NoResult`. Triaged as it is, that buries the board's own verdicts. Keep the `e<i>:Fail` (and other
      status) rows whose *i* has a record and drop the rest: 873 named failing cases, 187 groups.
    - **A slice with no console text names only its Fails.** Measured: of the 1,851 board fails 873 are named; the
      530 NotSupported and 12 other cases have counts only, and 25 CRCs stay unresolved (3 + 18 + 4 in the three
      `results.tsv` files, printed as `# e<i>: unresolved …` comments) because they belong to cases outside the
      row's filter. Naming the NotSupported cases needs a console or a `cases.tsv`, which this board cannot yet
      produce.
    - Against the HOST-ONLY `vivante` column of `docs/es3/host-gles3-tip.tsv`: 263 of the 873 fall in groups where
      the host also fails and never passes (GL core or shader-compiler side), 1 is board-only
      (`functional.lifetime.attach.deleted_output`, e22), the rest read `host-mixed` or `host-unknown` because the
      host table is per two-level group (`functional.fbo`, `functional.shaders`) and those are render groups the
      host cannot judge. `--depth 4` groups are finer than the host rows, so a group with 0 in both host columns
      is the host saying nothing, not the host passing.

15. **Results drafts** (edits no file):
    ```sh
    python3 -I tools/cts/imx8mp-gles3-report.py --date <YYYY-MM-DD> $D
    ```
    Runs `--check` and `--totals` and prints four headed blocks: the RESULTS.md entry, the STATUS.md line
    (Japanese), the ROADMAP stage 4 line and the text of the GitHub status comment. Exit 0 when the check passed;
    exit 1 marks the drafts PARTIAL and names the problems. The drafts go into RESULTS.md, STATUS.md and
    ROADMAP.md only in the final results item, not in a slice commit.

16. **An entry that overruns the window** (a watchdog end: `begun` below the mustpass count, `back_after` near
    600 s plus boot, the scan's `NO FINISH row <n> (watchdog end)` after `gc_wdog_arm: feeding bound ... over`).
    Try a split on the host first:
    ```sh
    python3 -I tools/cts/imx8mp-gles3-gen.py --split functional.texture.wrap=4 > trial-table.txt
    python3 -I tools/cts/check-imx8mp-gles3-tree.py -q --table trial-table.txt
    ```
    `--split <group>=<N>` (repeatable; the group is a dEQP group without `dEQP-GLES3.`) caps the per-entry
    budget of every case under the group at ceil(cases under the group / N); an unknown group or a count below 1
    exits with `--split ...: no such group or bad part count`. For the real change add the group to `SPLIT` in
    `tools/cts/imx8mp-gles3-gen.py`, regenerate with `python3 -I tools/cts/imx8mp-gles3-gen.py >
    tools/cts/imx8mp-gles3-tree.txt`, run `python3 -I tools/cts/check-imx8mp-gles3-tree.py -q` (expected `...
    matched exactly once 42810` and `OK`; without `-q` it prints `entry <i> <chars> chars <n> cases <first
    pattern>` per entry) and commit the generator and the table together. A re-split renumbers every later entry
    and changes the table image: slices already recorded stay evidence for the old table, and the campaign
    restarts from step 1 on the new commit.

`tools/cts/cts-selftest.sh` runs the selftests of the merge, pipeline, gen, wrap and triage tools; the preflight
and report selftests (`imx8mp-gles3-preflight-selftest.sh`, `imx8mp-gles3-report-selftest.sh`) are not part of it.

## RGBA32 two-plane sampling run (one session)

Once the flip (RGBA32F/UI/I in `GC_SAMPLED_TEXFMTS`, items gc-w128-flip and gc-w128-guard) is on `origin/main`, the
GC7000UL samples these formats as two G32R32 planes. Nothing of this has run on the board. The run needs one
session, one commit and one GL client at a time, under the rules of "Running a GLES3 slice session". Results go in
the board session, not in the commit that adds this section.

1. **Build from a commit with the flip.** `git fetch origin`, then `COMMIT=$(git rev-parse origin/main)` and
   `HEAD8=${COMMIT:0:8}`, only when `git log origin/main` shows the flip and the guard. Check that
   `git rev-parse HEAD` equals `$COMMIT`. Both images below come from this one commit. Before the flip the first
   four sub-checks of step 2 fail by design.

2. **The `w128tex` case** (gl_test case 244; code, values and check numbering in
   `driver/tools/pvrlab/gl_test_es3_w128tex.inc`, described in `driver/tools/pvrlab/README.md`):
   ```sh
   tools/cts/imx8mp-gltest-build.sh w128tex-$HEAD8 244
   SEL=0 BOARD_BOOT=uboot-mmc tools/board.sh imx8mp run $IMX8MP_IMG_OUT/w128tex-$HEAD8/zephyr/zephyr.bin <logdir> w128tex
   ```
   `BOARD_HOST`, `BOARD_UART` and the output directories come from the environment. Expected verdict lines:
   ```
   gl_test: w128tex: rgba32f sampler2D: PASS (16 of 16 texels)
   gl_test: w128tex: rgba32ui usampler2D: PASS (16 of 16 texels)
   gl_test: w128tex: rgba32i isampler2D: PASS (16 of 16 texels)
   gl_test: w128tex: rgb32f sampler2D, alpha 1.0: PASS (16 of 16 texels)
   gl_test: w128tex: swizzle refused: PASS (0 of 4096 pixels changed, GL_NO_ERROR 0x0)
   gl_test: w128tex: 5 of 5 sub-checks pass
   ```
   With no console, the CASE record carries the first bad check: 1-16 rgba32f, 17-32 rgba32ui, 33-48 rgba32i,
   49-64 rgb32f, 65 the swizzle draw's GL error, 66 the pixels it changed. Settle a failure here before step 3: the
   failing sub-check names the format whose planes are wrong. If the swizzle key has landed, sub-check 5 is a drawn
   check (R and B swapped) and the case's README section says what it prints.

3. **The dEQP case list.** The flip reaches 178 dEQP-GLES3 cases (56 sized, 122 specification); 51 RG32I/UI
   cases sit beside them and are not expected to move. They are table entries 70-80 and 94-103 of
   `tools/cts/imx8mp-gles3-tree.txt` (entries count from 0, as `TREE_ROWS` does). Check the numbers against the
   table of the commit you build, with `python3 -I tools/cts/imx8mp-gles3-sessions.py` (each entry's first pattern
   names its format). Build the table image (slice session step 4), then run the rows as in step 10:
   ```sh
   TREE_ROWS=70-80 TREE_IMAGE=$IMX8MP_OUT/es3-$HEAD8/zephyr/zephyr.bin \
     tools/cts/imx8mp-gles3-board.sh tree records/imx8mp/es3-$HEAD8-rows70-80 2>&1 | tee records/imx8mp/es3-$HEAD8-rows70-80.log
   TREE_ROWS=94-103 TREE_IMAGE=$IMX8MP_OUT/es3-$HEAD8/zephyr/zephyr.bin \
     tools/cts/imx8mp-gles3-board.sh tree records/imx8mp/es3-$HEAD8-rows94-103 2>&1 | tee records/imx8mp/es3-$HEAD8-rows94-103.log
   ```
   Then the console scan, the verdict reading and the commit of steps 10-12. Expect the sized and specification
   RGBA32 cases to move to pass, with zero recoveries. Still red whatever this run shows (RGBA32 render targets,
   not sampling): `fbo.color.*32*` (60 cases), `fbo.blit.conversion` (128) and `pbo.renderbuffer` (12). An entry
   that overruns the window is split as in step 16.

4. **Where the results go** (in the board session). The `w128tex` verdict lines and the tallies of the two row
   ranges replace their rows in `tools/cts/RESULTS.md` (current results only, no run log) and the summary in
   `tools/cts/STATUS.md`. A comment on GitHub issue #24 carries the commit and the result; the RC2 clause of
   `docs/es3-cts-failures.md` then moves from "host-done, i.MX8MP board run pending" to the measured result.

## ES 2.0 regression rows

These are separate and unchanged. Build with the gles2 stage 1 and no `DEQP_MODULE`, then run
`tools/cts/imx8mp-full-tree.txt` (29 rows) as described in `tools/cts/README.md`. Expect 0 failures and rows as
in `tools/cts/imx8mp-full-tree-29460f96.results.tsv`.

```sh
DEQP_STAGE1=<gles2 stage 1> tools/cts/imx8mp-deqp-build.sh wt-$HEAD8 'dEQP-GLES2.info.*' --table tools/cts/imx8mp-full-tree.txt
TREE_IMAGE=$IMX8MP_OUT/wt-$HEAD8/zephyr/zephyr.bin tools/board.sh imx8mp tree tools/cts/imx8mp-full-tree.txt records/imx8mp/es2-$HEAD8
```
