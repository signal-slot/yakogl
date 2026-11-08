<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# dEQP-GLES3 on the Verdin i.MX8M Plus (GC7000UL): board recipe

**First run: 2026-11-07/08 on main `cd9ebb5c`, all 133 entries, on the crash-route kexec from Torizon
(`BOARD_BOOT=kexec-dev`) without a console** (`tools/cts/log/2026-11-07-imx8mp-es3/README.md`, which
also says what that route needs: the board's demo container stopped before each boot, and a name page
from 32 on reads as the status selector, so pages stop at 31). The sections below are still the
U-Boot route as written before it; the board's own lines there are unverified. Board results are recorded under `tools/cts/log/<date>-imx8mp-gles3-<slice>/`; open work is
issue #2 of signal-slot/yakogl. The commands are
the existing gles2 ones plus the gles3 switch. The expected lines have the shape of real gles2 lines from
the i.MX8MP gles2 runs, which went through the same `board.sh` code path. The first board session
replaces them with what it sees.

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
Measured: 453 ninja steps, about 15 minutes on a 12-thread worker, 2.0 GB.

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

The footprint `board.sh` checks is the AArch64 header's `image_size` (`nb_check_image` in
`tools/uboot-netboot-lib.sh`): Zephyr writes `_flash_used` there, the distance from `0x60000000` to the
end of `.last_section`, which this image links right after `.last_ram_section`. It is 227,909,632 bytes
(222,568 KB, the linker's `RAM` line), 84.9 % of the 256 MiB `a53_ddr_section`; `board.sh` refuses an
image whose `image_size` passes `0x70000000`, and on the `kexec-dev` route one larger than the crash
window. `CONFIG_KERNEL_VM_SIZE=0x20000000` (512 MiB) is twice the region, so it covers the image.

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
  `e00`..`e132`, selectors 0..132, `footprint 227909632`.
- **Subset:** `TREE_ROWS=0-9` or `TREE_ROWS=57,60-66` runs only those rows.
- **Resume:** a row whose run directory already has an `INDEX.txt` is skipped.

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
`NAMES=0` turns this off.

**Failing names.** `board.sh` maps each CRC to a name with `yakogl-tools record crc <c>`. Its case list
is `tools/cts/gles2-mustpass.txt` and `tools/cts/gles3-mustpass.txt` together and it matches a check
against every name in it, so a GLES3 table image's failures resolve as they are: the `failing names`
column of `results.tsv` holds case names, never `crc:<hex>` — kept only when they start inside the row's
own table entry (column 2). A name check of two names can be equal; both are then printed. To search one
module or another list only: `record crc <hex> dEQP-GLES3.`, or `DEQP_CASELIST=<file>` to replace the
list.

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
  has `zephyr,console = &uart3`) after the table fragment. Only the value `1` counts. It was checked on the
  host only (the selftest, no Zephyr build), so the first real build is the check that Zephyr accepts it.
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
- **Unverified until a board session:** that the console prints at all with the image used, that the
  capture is complete for a 600 s row, and that the capture does not disturb the run.

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

## Running a GLES3 slice session

One ordered procedure for a board session that runs a row range of the 133-entry table and commits it as a
slice. Every command below exists in the tree with the flags shown, and the output described is what its script
prints (read from the script). Nothing here has run on the board yet, so what the board prints is marked
**unverified** where it matters.

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
  as it is and rerun; it is not a clean row.
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
   `BOARD_UART`. `tools/cts/imx8mp-env.sh` is sourced by the build scripts, not run. It needs `ZEPHYR_WS` (the
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
   three lines (**unverified**: the first real build is the check; if one is missing, stop and report).

5. **Artefacts for the session note:**
   ```sh
   tools/cts/imx8mp-gles3-artefacts.sh es3-$HEAD8
   ```
   Prints `<bytes> <sha256> <path>` for `zephyr.bin`, `zephyr.elf`, `es3-$HEAD8.table.conf`,
   `libdeqp-gles3-package.a` and `libdeqp-gles3-functional.a`, then the two `.config` lines for
   `CONFIG_DEQP_REPORT_ES2` and `CONFIG_YAKOGL_REPORT_ES2` (`# ... is not set` expected; `... not in .config` if
   absent) and the linker's `Memory region`/`RAM:` lines. It exits non-zero when a file is missing. These lines
   go into the slice README.

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
   and run them last, in small ranges.

7. **Preflight** (read-only; no network, no board):
   ```sh
   tools/cts/imx8mp-gles3-preflight.sh --commit "$COMMIT" --out "$IMX8MP_OUT"      # also: --table <file>
   ```
   One line per check, `  ok   <text>` or `  FAIL <text>`: `IMX8MP_OUT`, `DEQP_STAGE1`, `ZEPHYR_WS` and
   `HAL_NXP_DIR` set and existing, `BOARD_UART is set`, the stage 1 package, `table passes
   check-imx8mp-gles3-tree.py`, `table image exists (es3-<hash8> ...)`, `image es3-<hash8> is commit <hash8>`.
   The last line is `preflight: all ok` (exit 0) or `preflight: <n> check(s) FAILED` (exit 1); a bad option exits
   2. `--commit` defaults to `git rev-parse origin/main`, so pass the campaign commit once main has moved. Run
   it before every slice. Its selftest covers stub files only: the stage 1 and image checks run for real here
   for the first time.

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
    TREE_ROWS=0-9 TREE_IMAGE=$IMX8MP_OUT/es3-$HEAD8/zephyr/zephyr.bin \
      tools/cts/imx8mp-gles3-board.sh tree "$OUT" 2>&1 | tee "$OUT.log"
    ```
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
    `done_with_resets`, `died_after=<stage>`, or `back_after` near 600 s plus boot with `begun` below the entry's
    mustpass count (a watchdog end: step 16). Rerun a row that is not clean once; if it repeats, record it as it is
    and treat it in step 16 or the triage. Note each row's `back_after` and scan elapsed time.

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

14. **Not-pass list, totals and triage** (after step 13 passes without `--partial`):
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

## ES 2.0 regression rows

These are separate and unchanged. Build with the gles2 stage 1 and no `DEQP_MODULE`, then run
`tools/cts/imx8mp-full-tree.txt` (29 rows) as described in `tools/cts/README.md`. Expect 0 failures and rows as
in `tools/cts/imx8mp-full-tree-29460f96.results.tsv`.

```sh
DEQP_STAGE1=<gles2 stage 1> tools/cts/imx8mp-deqp-build.sh wt-$HEAD8 'dEQP-GLES2.info.*' --table tools/cts/imx8mp-full-tree.txt
TREE_IMAGE=$IMX8MP_OUT/wt-$HEAD8/zephyr/zephyr.bin tools/board.sh imx8mp tree tools/cts/imx8mp-full-tree.txt records/imx8mp/es2-$HEAD8
```
