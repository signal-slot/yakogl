<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# STM32MP257F-DK P10: direct boot from the bootloader, and the `tools/board.sh` sign-off

Roadmap item P10 (`docs/notes/pre-study-stm32mp257f-dk.md`, the P10 row after
P9): boot the DK's images straight from its own bootloader and sign the board
off through the common runner `tools/board.sh`, as the other boards are. This
note is the procedure and the gap list, written on the host from the tree as it
stands after P9, before any P10 board session. **Status: nothing here has run
on the board for P10.** What the board has already shown for the boot paths
is cited from its notes, and what is not shown is in section 7.

Citations are `file:line` on the tree this note was written on. The note
carries no board address, host name, console path or serial: those come from
the environment (`BOARD_HOST`, `BOARD_UART`) and the farm's leases only.
`tools/board.sh` belongs to plans #396/#397 until they merge, so this note
edits no runner; section 4 says what the row needs from it.

## 1. What "direct boot" means here, and which path it takes

The i.MX8MP's and the Pi's "direct boot" replaces or skips the vendor
bootloader (`docs/notes/am62p-direct-boot.md`, `docs/notes/imx8mp-direct-boot.md`
use `driver/samples/zephyr/boot_stage2`). **That is not P10.** The pre-study
keeps the DK's chain as ST built it: TF-A v2.10 BL2/BL31, OP-TEE 4.0, ST's
U-Boot 2023.10, and the replacement of U-Boot proper is "a later item"
(`docs/notes/pre-study-stm32mp257f-dk.md`, "Boot route for sign-off"). For
P10 a direct boot is:

> ST's own U-Boot, on the boot that follows a reset, loads the image from the
> microSD's bootfs and `go`es to it at `0xa0000000` (EL2, MMU and caches off,
> `docs/notes/pre-study-stm32mp257f-dk.md` Q8), with no kexec, no jump from
> Linux, and nobody typing at the U-Boot prompt; the image's end (PSCI reset
> or OP-TEE's watchdog) puts ST's Linux back.

The FIP (fip-a, fip-b), BL2 (fsbla1/2), the FWU metadata and U-Boot itself are
never written. Three paths in the tree meet that definition's neighbours;
the table says which is P10's:

| Path | Armed by | Image on bootfs | Runner | Is it "direct"? |
|---|---|---|---|---|
| **typed one-shot** (`BOARD_BOOT=uboot-mmc`, board.sh's default for the DK) | nothing stored: the runner types one line at ST's prompt over the console, `mp2z_*` variables in RAM only | `/boot/yakogl/mp2z-<token>.{scr,bin}` | `tools/board-stm32mp2-uboot.sh` (`:3-48`) | no: the console is written to; it is the typed route of `docs/notes/stm32mp257f-dk-uboot-boot.md` |
| **env-armed one-shot** (`BOARD_BOOT=standalone` in board.sh) | `preboot=run yako_hook`, then per run `yako_once` and `yako_file`, in the `u-boot-env` partition, edited from the host by `tools/uboot_env.py` | `/boot/yakogl/once-<token>.bin` | `tools/board-stm32mp2.sh` (`:56-92`) | **yes, this is P10's sign-off route**: U-Boot's own `preboot` runs before the autoboot countdown, the console is only read, and every run P6-P9 used it |
| **counted default** | `boot_prefixes` gets `/yakogl/standalone/` in front, plus `mp2s_*` | `/boot/yakogl/standalone/{boot.scr.uimg,default.bin}` | `tools/board-stm32mp2-standalone.sh` (`:1-60`) | **yes, the product form**: runs N boots from power-on with no host (`docs/notes/stm32mp257f-dk-standalone.md`, the stage-5 "standalone boot"), then Linux |

Decision for P10 (an assumption, section 7): the sign-off *row* is the
env-armed one-shot, because it is what P6-P9 ran, it needs no console input,
and its undo is a single variable set; the counted default is run once with the
P10 image as the product-form proof (`default <image> 2` as on 2026-11-23,
`docs/notes/stm32mp257f-dk-standalone.md`, "Board run"), not as a row.
`forever` is never used without a person (`BOARD_PERSON_PRESENT=1`).

### The variables each path changes

| Path | Stored variables it writes (U-Boot `u-boot-env`) | Undo, printed before the change |
|---|---|---|
| env-armed one-shot | `preboot` (only when the stock environment has none; a preboot of another value is never overwritten: rc 7), `yako_hook`, `yako_mmc`, `yako_addr`, `yako_part`, `yako_bootcmd_prev` at `install`; `yako_once`, `yako_file` per run (U-Boot deletes `yako_once` and saves before it loads anything) | `uninstall` (deletes `preboot` when it is exactly `run yako_hook`, and every `yako_*`); `disarm` clears a pending one-shot and staged images; the pristine environment is kept on the host (`records/stm32mp257f-dk/uboot-env/`) and on the card (`/boot/yakogl/backup/u-boot-env.bin`) |
| counted default | `boot_prefixes`, `mp2s_ver`, `mp2s_left`, `mp2s_prefixes` (the inactive copy, flags + 1, as U-Boot's `saveenv` does) | `nodefault`; `uninstall`; at the prompt `setenv mp2s_left 0; saveenv; boot`; no console: the card in a PC and `tools/mp2s_card_undo.sh --yes <bootfs partition>` |
| typed one-shot | none stored | the typed line's first command echoes the undo; `disarm` removes staged files |

`bootcmd` is never written by any path (it is ST's; the `bootcmd` fallback
prefix of the env-armed route is typed by a person only when `preboot` is
unavailable, and `uninstall` then stops with NEEDS A PERSON until it is
put back).

## 2. The board constraints (binding)

These bind every step, here and in the board sessions of section 6. They are
the runners' own guards (`dev_guard`, `card_check`, `guard_path`;
`tools/board-stm32mp2.sh:140-170`, `tools/board-stm32mp2-standalone.sh:64-76`,
`docs/notes/stm32mp257f-dk-uboot-boot.md` section 10):

- **Never write** the partitions `fsbla1`, `fsbla2`, `metadata1`, `metadata2`,
  `fip-a` or `fip-b`; the FWU metadata is only read. Never write `vendorfs`,
  `rootfs` or `userfs`.
- **Never write the eMMC**, and never read it: the eMMC is the disk whose
  `/sys/block/<disk>/device/type` says `MMC`, never a `mmcblk` number (Linux
  numbers the card and the eMMC differently from boot to boot) and never
  `/dev/disk/by-partlabel`, which names the eMMC's partitions.
- **Never repartition** (`gpt`, `part`, `sgdisk`), create a file system, or
  load or unload a kernel module; never touch Tailscale or sshd.
- **Change only** (a) files under `/boot/yakogl/` on the card's bootfs (never
  a stock file under `/boot`: `boot.scr.uimg`, the extlinux files, `Image.gz`
  and the device trees are checked unchanged against `boot-sha256.txt`), and
  (b) the runner's own U-Boot variables of section 1 in `u-boot-env`, by one
  `dd` of one copy, read back byte for byte, the pre-edit bytes put back at
  once on a mismatch. **Each change has its undo printed first**; a step whose
  runner has not printed the undo does not run.
- A result that looks like a hardware fault is checked against the USB-C
  supply first: BL2's `Reset reason:` line (`Power-on reset`, `Brownout
  reset` mean the supply; `IWDG system reset` means the watchdog)
  (`docs/notes/stm32mp257f-dk-bringup.md`, "When it looks like a hardware
  fault: supply first").
- Images are built from origin/main commits only, run from a detached
  checkout, with the commit in the record (`tools/cts/README.md`, board rules).

## 3. What a P10 run is, end to end

For the env-armed route (the runner's own header, `tools/board-stm32mp2.sh:171-200`):

1. Host: the image check (AArch64 `Image` header, footprint inside
   `0xa0000000..0xb0000000`, the marker `gc_wdog_arm: armed`).
2. Board lock and pre-checks (ssh, no foreign session, the hook as written,
   no one-shot pending, free space on bootfs, the console free); the undo is
   printed here.
3. Stage `/boot/yakogl/once-<token>.bin`, read back by sha256 after dropping
   the page cache.
4. Start the console capture (`BOARD_UART`, read only).
5. Arm `yako_once` and `yako_file`, read back with `dd` and decoded on the
   host; from here any exit disarms.
6. Reboot. U-Boot's `preboot` loads the image, `go`es to it; the image ends in
   `mp2: <why>: PSCI SYSTEM_RESET at <n> ms` (or OP-TEE's watchdog, about 32 s
   after a hang); Linux is back about 35 s after the reset.
7. Check the one-shot is gone, remove the staged image, write the record,
   print the verdict line.

The image is bounded by `CONFIG_YAKOGL_STM32MP2_LIFETIME_S` (20 s default,
100 s for gl_test, 600 s for dEQP, `docs/notes/stm32mp257f-dk-p9.md` section 2)
and the runner's `RUN_BOUND`/`RETURN_BOUND`.

## 4. What `tools/board.sh` needs for a `stm32mp257f-dk` row

`tools/board.sh` already knows the DK: the board table entry
(`tools/board.sh:533-548`: `YAKO_ADDR=0xa0000000`, `LOAD_END=0xb0000000`,
`MARKER='gc_wdog_arm: armed'`, `END_RE_DEFAULT='^mp2: .*: PSCI SYSTEM_RESET at [0-9]+ ms'`,
`LEGACY_RE` empty), the typed route's dispatch (`:2583-2599`, `:2663-2672`),
and `BOARD_BOOT=standalone`, which hands `run` and `status` to
`tools/board-stm32mp2.sh` (`:304-322`). What a P10 row needs on top, in the
order the board.sh steps run (`tools/board.sh:92-120`):

| Step | What it needs for the DK | State today |
|---|---|---|
| 1 image check | the DK window and marker | wired (`:533-548`) |
| 2 lock, lease, pre-checks | the runner takes the board lock itself, board.sh takes only the lease (`mp2_lease`, `:2669-2672`, `HOLD_MAX` 2100 s); the pre-checks are the runner's | wired for the typed and the standalone route |
| 3 clear the result sink | none: the DK has no SNVS-like sink, the console is the record | n/a |
| 4 stage and read back | the runner's, by sha256 | the runner's |
| 5 console capture | `BOARD_UART` read only (the typed route also writes it) | the runner's |
| 6 boot | `BOARD_BOOT=standalone`: the armed one-shot; no board.sh code arms it | wired as an exec; install is a direct call |
| 7 wait for the end | `END_RE` above, bound `RUN_BOUND` + `RETURN_BOUND` | the runner's |
| 8 read the records | the DK's image prints `yk-record:` lines (`driver/zephyr/lib/board/stm32mp2/board_stm32mp2.c:29`) | not read by either DK runner; their `result=` is the image's final PASS/FAIL line |
| 9 the verdict line | board.sh's format, section below | the DK runners print a shorter variant |
| 10 clean up | one-shot gone, staged image removed | the runner's |

**The verdict line.** board.sh's shape (`tools/board.sh:134-139`) is
`board.sh <board> <name> rc=<rc> verdict=<v> boot=<backend> <row|DEV>
boot_verdict=<bv> result=<record:KIND|legacy:end line|none> resets=<n|absent>
record=<w0,w1,w2,w3|absent> back_after=<s>s`. The env-armed runner prints
`... boot=uboot-sd <row|blind> boot_verdict= result=<pass|fail:<prog>|none>
back_after=` (`tools/board-stm32mp2.sh:269-276`), the typed runner
`... boot=<uboot-mmc|uboot-tftp> row boot_verdict= result= env_diff= boot_diff=
back_after=` (`tools/board-stm32mp2-uboot.sh:123-130`). A sign-off row needs one
shape. The verdict words that count as a pass are `done` (rc 0) and, for the
hang trial only, `watchdog_reset` (rc 0). `returned` (a blind run) and
`done` with `row_eligible=no` are never a row.

**The record layout.** `<logdir>/<YYYYMMDD-HHMM>-<name>-<sha8>/` with
`meta.txt` (key=value, the `left=` lines of an undo that was cut off),
`console.txt`, `INDEX.txt` (build, image sha256 and build line, times, the
verdict line, every file with its sha256 and line count) and, from the typed
runner, `verdict.txt` (`tools/board.sh:161-170`; `tools/board-stm32mp2.sh:1653`).
Copies are redacted (IPv4, MAC, host names and the console path or bridge become
placeholders; `BOARD_HOST` is never written). For the DK the directory sits
under the gitignored `records/stm32mp257f-dk/`; no record file is committed
(CLAUDE.md: findings go into commit messages and notes). The env-armed runner's
stamp is `HHMM` (`:1653`), the typed runner's `HHMMSS`.

**The reset, by resource.** No runner and no board.sh code power-cycles the
DK. A board that does not come back is `verdict=not_back`, rc 5 (env-armed) or
rc 7 (typed), plus a `NEEDS A PERSON` line, and the runner never retries. In a
farm session the person is the lease: `resource:stm32mp257f-dk-reset`
(power-cycle) is used by the *session*, once, after the runner has released
the board; the session then runs `disarm` and `status` and reads BL2's
`Reset reason:` before anything else is attempted. The DK's only supply is its
USB-C cable, so a power-cycle is a supply event in the next boot's BL2 line
and is told apart there (section 2).

## 5. The gaps, and where each one lives

Each gap is a change to a file; none of them is made by this note.

| # | Gap | File | Owner |
|---|---|---|---|
| G1 | `BOARD_BOOT=standalone` forwards only `run` and `status`: `install`, `uninstall`, `disarm`, `env-edit`, `probe` and `--expect-watchdog` are refused with a pointer to a direct runner call (`tools/board.sh:319-321`; pinned by `tools/board_selftest.sh:448-451`). A row's pre-conditions (hook installed) are therefore not reachable through board.sh | `tools/board.sh` | #396/#397 |
| G2 | The DK's default `run` (`BOARD_BOOT` unset) is the typed route, which writes the console; a P10 row must name `BOARD_BOOT=standalone` every time, and the verdict line names `boot=uboot-sd`, which is not in board.sh's `BOOTS` for the DK (`uboot-mmc uboot-tftp`, `:535-536`) | `tools/board.sh`, `tools/board-stm32mp2.sh` | #396/#397 |
| G3 | Three verdict-line shapes (section 4); `resets=` and `record=` are absent on the DK lines. The DK layer prints `yk-record:` lines but neither DK runner decodes them (`yakogl-tools record --line`), so `result=record:<KIND>` cannot be produced | `tools/board-stm32mp2.sh`, `tools/board-stm32mp2-uboot.sh`, `tools/board.sh` | #396/#397 for board.sh; the DK runners are P10's |
| G4 | `tally` and `diff` are not DK verbs in board.sh (`:336-345`); the DK's tally is `tools/board-stm32mp2.sh tally <rundir>`. `tree` is refused for the DK (`:2988`): the DK's dEQP and gl_test sweeps are driven by `tools/cts/mp2-deqp-sweep.sh` and `tools/cts/mp2-gltest-shards.py` | `tools/board.sh` | #396/#397 |
| G5 | The counted default has no `run`-like verb: `default <image> N` reboots nothing and reads nothing; the reboot, the N-boot console scan and the come-back check are `tools/mp2s_session.sh`, `tools/mp2s_console_watch.py`, `tools/mp2s_record_check.py` and `tools/mp2s_comeback.sh`, none called from board.sh. Its proof is one session, not a row | `tools/board.sh`, `tools/board-stm32mp2-standalone.sh` | P10 session; board.sh part #396/#397 |
| G6 | No reset hook: a `not_back` leaves the board to a person (section 4). The session needs the reset resource written into its procedure, and the runner's NEEDS A PERSON line needs to say what the next step is | the session procedure (this note, section 6); `tools/board-stm32mp2.sh` message | P10 |
| G7 | The two runners' headers disagree on the card's device name (`mmcblk0` in `tools/board-stm32mp2.sh:104-107`, `mmcblk2`/`mmcblk0` varying in the standalone note): the code goes by `device/type`, so this is a comment only | `tools/board-stm32mp2.sh` | any |
| G8 | `tools/board_selftest.sh` covers the DK's dispatch with stubs (`:354-461`) but no P10 row: once G1-G3 move, its cases move with them | `tools/board_selftest.sh` | #396/#397 |
| G9 | A `watchdog_reset` run (the hang trial) is not a board.sh flag (`tools/board.sh:2589-2592`: "call the runner directly") | `tools/board.sh` | #396/#397 |

G1, G2, G3, G4, G8 and G9 are in files that #396/#397 own: they are requests to
those plans, not P10 edits. G5-G7 are P10's.

## 6. The order of the board sessions

All on `resource:stm32mp257f-dk` with `resource:stm32mp257f-dk-console`
(serial) and `resource:stm32mp257f-dk-reset` (power-cycle); one session at a
time (the board lock and lease), images from origin/main only, P9's results
first.

1. **Preflight, no change** (`status`, and `probe`): the hook state, the
   stored environment decoded on the host (both copies' CRCs), `fwu: bank 0
   accepted`, free space on bootfs, `/boot` on the SD disk by `device/type`,
   the stock boot files against `boot-sha256.txt`. Reads only.
2. **The env-armed one-shot as the sign-off route**: `install` (undo printed
   first, one environment write), then `mp2_hello` (return trial), `gc_triangle`
   and `gl_test` case 1 through `tools/board.sh` with `BOARD_BOOT=standalone`
   once G1-G3 allow it, otherwise through `tools/board-stm32mp2.sh run`
   directly with the verdict line as it is. One verdict line each, records
   under `records/stm32mp257f-dk/`.
3. **The hang trial**: `run --expect-watchdog` with the `mp2_hello` hang build
   (`verdict=watchdog_reset`; the board must be back in about 32 s plus
   Linux's boot).
4. **The counted default**, once, with a P10 image: `default <image> 2`,
   `systemctl reboot` by the session, the console read for both image boots
   and the third (Linux), `mp2s_console_watch.py scan`, `mp2s_record_check.py
   --count 2`, `mp2s_comeback.sh --before-boot-id`, `nodefault`, `uninstall`,
   and the `/boot` listing compared with the one before (`mp2s-boot: same`).
   No `forever`.
5. **The sign-off set through board.sh**: the same images as P9's slices, a
   row each (`gl_test` set, dEQP shards as the sweep tool splits them), after
   G1-G4 are closed, with the board.sh verdict lines in the notes' "Board
   verdicts" and `tools/cts/RESULTS.md` as plan #392 owns it.
6. **The undo walk**: `uninstall` (stock boot files `same`, `preboot` and the
   `yako_*` variables gone, the environment equal to the pristine image apart
   from the loop's working variables). The board is left as found.

After a `not_back` or a NEEDS A PERSON at any step: stop the session; power-cycle
once through the reset resource; run `disarm` and `status`; read BL2's
`Reset reason:`; do not retry the step in the same session.

## 7. What is not verified

- **Nothing of P10 has run.** The env-armed one-shot has run on the board for
  P6-P9 (`docs/notes/stm32mp257f-dk-bringup.md`, "Board verdicts";
  `docs/notes/stm32mp257f-dk-p9.md` section 6), the counted default once
  (`docs/notes/stm32mp257f-dk-standalone.md`, "Board run", 2026-11-23); the
  typed route is read from the code and its notes. None of them has run
  *through* `tools/board.sh` on the DK: only the dispatch is tested, with
  stubs (`tools/board_selftest.sh:354-461`).
- The choice of the env-armed one-shot as the row (section 1) is this note's,
  not the owner's. If the owner wants the counted default as the row, G5 grows
  into a board.sh verb.
- Whether a board.sh row for the DK should carry the typed route's `env_diff=`
  and `boot_diff=` fields is open; the env-armed runner reports the same facts
  as `meta.txt` lines instead.
- Whether the DK's `yk-record:` line survives the console capture well enough to
  be the row's `result=record:` (G3): no run has decoded one.
- Whether ST's `preboot` runs on every DK software release: it did on the
  card flashed with OpenSTLinux v26.06.10 (U-Boot 2023.10-stm32mp-r3.1);
  another release was not seen.
- The reset resource has not been used by any DK session in these notes; its
  effect on BL2's reset reason and on the card's state is not recorded.
- The i.MX8MP's netboot shape (`docs/notes/imx8mp-netboot.md`: TFTP, SNVS) has no
  DK counterpart beyond the typed `uboot-tftp` route, which has not run on the
  DK at all.
- The residual: P8's display (after P10) needs the same boot path; its
  timeouts are not part of this note.

Next after P10: P8's display (`docs/notes/stm32mp257f-dk-display.md`), and the
roadmap's remaining stage-5 items.
